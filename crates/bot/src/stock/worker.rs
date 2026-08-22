//! The close-report scheduler. A **separate** 60s `tokio` task (not merged into
//! `Scheduler`): different cadence, a clean `--dry-run`, and — most importantly —
//! isolation, so a stock DB error can't starve RSS delivery (and vice versa).
//!
//! The loop is log-and-continue: a transient upstream 500 must never kill the
//! task and, through `try_join!`, the whole process. Idempotency is the DB
//! ledger (claim → send → mark sent, at-most-once with bounded retry) plus a
//! process-lifetime in-memory guard so a persistently-failing stamp resends at
//! most once per process rather than every 60s.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::watch;
use tracing::warn;

use crate::bot::render::chunk_lines;
use crate::bot::runtime::chat_lang;
use crate::bot::sender::{MessageSender, SendOptions, SendOutcome};
use crate::config::MessageMode;
use crate::db::bookmarks::now_unix;
use crate::db::stocks::{REPORT_CLAIMED, REPORT_SENT};

use super::clock::{
    classify_session, decide_push, market_date_string, PushDecision, PushTime, SessionMeta,
    SessionState,
};
use super::render::{render_daily_report, ReportEntry};
use super::service::StockService;
use super::source::StockSource;
use super::symbol::{self, Market, Parsed, Symbol};

const POLL_SECS: u64 = 60;
/// Re-probe the market clock at most this often; between probes a cached
/// `SessionMeta` is reused so the ~1439 no-push ticks per day cost no network.
const PROBE_TTL_SECS: i64 = 300;
/// Hard cap on symbols rendered in one report; the daily report is not a data
/// dump. Overflow gets a "…and N more" line.
const MAX_REPORT_ENTRIES: usize = 30;
const CLAIM_STALE_SECS: i64 = 1800;
const CLAIM_MAX_ATTEMPTS: i64 = 3;

use crate::bot::i18n::Lang;

/// Renders a report to size-bounded message chunks (≤3), appending an overflow
/// note when more symbols exist than we show. Shared by the worker and the
/// manual `/stockreport`.
pub fn render_report_chunks(
    market: Market,
    trade_date: &str,
    entries: &[ReportEntry],
    late: bool,
    lang: Lang,
) -> Vec<String> {
    let shown = entries.len().min(MAX_REPORT_ENTRIES);
    let mut lines = render_daily_report(market, trade_date, &entries[..shown], late, lang);
    if entries.len() > shown {
        lines.push(lang.stk_report_overflow(entries.len() - shown));
    }
    let mut chunks = chunk_lines(&lines, 3500);
    chunks.truncate(3);
    chunks
}

fn send_options() -> SendOptions {
    SendOptions {
        disable_web_page_preview: true,
        disable_notification: false,
        parse_mode: MessageMode::Html,
    }
}

fn probe_symbol(raw: &str) -> Option<Symbol> {
    match symbol::parse(raw) {
        Parsed::Resolved(sym) => Some(sym),
        _ => None,
    }
}

fn market_index(market: Market) -> usize {
    match market {
        Market::Tw => 0,
        Market::Us => 1,
    }
}

pub struct StockWorker<Src: StockSource, Snd: MessageSender> {
    stock: std::sync::Arc<StockService<Src>>,
    sender: Snd,
    /// Present only when AI commentary is enabled *and* the MCP bridge is
    /// configured; otherwise reports send the numbers alone.
    commentary: Option<crate::tagging::mcp::McpClient>,
    probe_cache: Mutex<[Option<(SessionMeta, i64)>; 2]>,
    sent_guard: Mutex<HashSet<(i64, Market, i64)>>,
}

impl<Src: StockSource, Snd: MessageSender> StockWorker<Src, Snd> {
    pub fn new(stock: std::sync::Arc<StockService<Src>>, sender: Snd) -> Self {
        Self::with_commentary(stock, sender, None)
    }

    pub fn with_commentary(
        stock: std::sync::Arc<StockService<Src>>,
        sender: Snd,
        commentary: Option<crate::tagging::mcp::McpClient>,
    ) -> Self {
        Self {
            stock,
            sender,
            commentary,
            probe_cache: Mutex::new([None, None]),
            sent_guard: Mutex::new(HashSet::new()),
        }
    }

    pub async fn run_until_shutdown(&self, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
        loop {
            // Log-and-continue: never `?` out of the loop (that would kill the
            // task and, via try_join!, the whole process). Contrast the
            // Scheduler, which propagates.
            if let Err(err) = self.run_once(now_unix()).await {
                warn!(error = %err, "stock worker pass failed");
            }
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_ok() && *shutdown.borrow() {
                        break;
                    }
                }
                () = tokio::time::sleep(Duration::from_secs(POLL_SECS)) => {}
            }
        }
        Ok(())
    }

    pub async fn run_once(&self, now: i64) -> anyhow::Result<()> {
        // Each market is independent: one market's outage never starves the
        // other (each warns and continues).
        for market in [Market::Tw, Market::Us] {
            if let Err(err) = self.process_market(market, now).await {
                warn!(?market, error = %err, "stock worker market pass failed");
            }
        }
        Ok(())
    }

    async fn process_market(&self, market: Market, now: i64) -> anyhow::Result<()> {
        let repo = self.stock.repo();
        // First query is a single cheap SELECT; empty means immediate return
        // with zero HTTP — the 99%-of-ticks path.
        let chats = repo.chats_with_market(market.as_wire()).await?;
        if chats.is_empty() {
            return Ok(());
        }
        let Some(probe) = probe_symbol(self.probe_symbol_raw(market)) else {
            return Ok(());
        };
        let meta = self.session_meta(market, &probe, now).await?;
        let session = classify_session(now, meta);
        // Only a finalized close is ever pushed. Open / Settling / NoSession /
        // NoTrade all return here with nothing sent.
        let SessionState::Closed { trading_day } = session else {
            return Ok(());
        };
        let trade_date = market_date_string(trading_day);

        for chat_id in chats {
            if let Err(err) = self
                .push_chat(chat_id, market, meta, session, now, trading_day, &trade_date)
                .await
            {
                warn!(chat_id, ?market, error = %err, "stock push failed for chat");
            }
        }
        Ok(())
    }

    fn probe_symbol_raw(&self, market: Market) -> &str {
        let cfg = self.stock.config();
        match market {
            Market::Tw => &cfg.tw_probe_symbol,
            Market::Us => &cfg.us_probe_symbol,
        }
    }

    /// The market clock, re-probed at most every `PROBE_TTL_SECS`.
    async fn session_meta(
        &self,
        market: Market,
        probe: &Symbol,
        now: i64,
    ) -> anyhow::Result<SessionMeta> {
        let idx = market_index(market);
        {
            let cache = self.probe_cache.lock().unwrap();
            if let Some((meta, at)) = cache[idx] {
                if now - at < PROBE_TTL_SECS {
                    return Ok(meta);
                }
            }
        }
        let meta = self.stock.fetch_session_meta(probe).await?;
        self.probe_cache.lock().unwrap()[idx] = Some((meta, now));
        Ok(meta)
    }

    #[allow(clippy::too_many_arguments)]
    async fn push_chat(
        &self,
        chat_id: i64,
        market: Market,
        meta: SessionMeta,
        session: SessionState,
        now: i64,
        trading_day: i64,
        trade_date: &str,
    ) -> anyhow::Result<()> {
        let repo = self.stock.repo();
        let cfg = self.stock.config();

        // Process-lifetime guard: if a prior pass already sent this day (and a
        // stamp write then failed), don't resend every 60s.
        if self.sent_guard.lock().unwrap().contains(&(chat_id, market, trading_day)) {
            return Ok(());
        }

        let setting = repo.get_push_setting(chat_id, market.as_wire()).await?;
        let enabled = setting.as_ref().is_none_or(|s| s.enabled != 0);
        let pref = setting.and_then(|s| s.push_minute).map(PushTime);
        let default_delay = i64::try_from(self.default_delay_minutes(market)).unwrap_or(60) * 60;
        let late_threshold = i64::try_from(cfg.late_threshold_minutes).unwrap_or(30) * 60;
        // An existing ledger row (any status) means "already handled today".
        let last_sent = repo
            .report_status(chat_id, market.as_wire(), trade_date)
            .await?
            .map(|_| trading_day);

        let decision = decide_push(
            now,
            session,
            meta,
            enabled,
            pref,
            default_delay,
            last_sent,
            late_threshold,
        );
        let PushDecision::Send { late, .. } = decision else {
            return Ok(());
        };

        // Claim (fresh or a bounded retry of a stuck claim). A `false` means
        // another instance/pass owns it.
        if !repo
            .claim_report(chat_id, market.as_wire(), trade_date, now, CLAIM_STALE_SECS, CLAIM_MAX_ATTEMPTS)
            .await?
        {
            return Ok(());
        }

        let lang = chat_lang(repo, chat_id).await;
        let mut entries = self.stock.report_entries(chat_id, market, trade_date, now).await?;
        if entries.is_empty() {
            // Nothing to report (all symbols removed since the chat list query).
            // Stamp the day so we don't reclaim it every stale window.
            repo.mark_report_sent(chat_id, market.as_wire(), trade_date).await?;
            return Ok(());
        }

        // Optional AI commentary — never on the critical path: any failure or
        // timeout leaves the numbers untouched.
        if cfg.ai_commentary {
            if let Some(client) = &self.commentary {
                if let Err(err) = super::commentary::annotate_entries(
                    client, repo, lang, trade_date, &mut entries, cfg,
                )
                .await
                {
                    warn!(chat_id, error = %err, "stock commentary annotate failed");
                }
            }
        }

        let chunks = render_report_chunks(market, trade_date, &entries, late, lang);
        let mut forbidden = false;
        for chunk in &chunks {
            match self.sender.send_text(chat_id, chunk, send_options(), None).await {
                Ok(SendOutcome::Sent) => {}
                Ok(SendOutcome::Forbidden) => {
                    // Never delete the watchlist (a blocked send is not a
                    // user-intent signal), but DO stamp the day so we stop
                    // retrying a blocked chat every 60s.
                    forbidden = true;
                    break;
                }
                Err(err) => {
                    // Transient: leave the claim so a later stale-window retry
                    // can resend. Do not set the guard.
                    warn!(chat_id, error = %err, "stock report send failed; will retry");
                    return Ok(());
                }
            }
        }

        // Sent (or forbidden) — record it, and guard against a stamp-write
        // failure resending within this process.
        self.sent_guard.lock().unwrap().insert((chat_id, market, trading_day));
        if forbidden {
            repo.mark_report_forbidden(chat_id, market.as_wire(), trade_date).await?;
        } else {
            repo.mark_report_sent(chat_id, market.as_wire(), trade_date).await?;
        }
        Ok(())
    }

    fn default_delay_minutes(&self, market: Market) -> u64 {
        let cfg = self.stock.config();
        match market {
            Market::Tw => cfg.default_delay_minutes_tw,
            Market::Us => cfg.default_delay_minutes_us,
        }
    }
}

/// Whether a manual `/stockreport` for a market should proceed, and for which
/// day. Returns `None` unless the market is closed (a report before the bell
/// would print intraday jitter as a close). Shared status constants are used by
/// the handler to phrase its reply.
pub fn manual_report_day(session: SessionState) -> Option<i64> {
    match session {
        SessionState::Closed { trading_day } => Some(trading_day),
        _ => None,
    }
}

/// Re-exported so the manual-report handler and tests share the ledger status
/// meaning without re-importing the db module.
pub const STATUS_CLAIMED: i64 = REPORT_CLAIMED;
pub const STATUS_SENT: i64 = REPORT_SENT;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::sender::test_support::RecordingSender;
    use crate::config::StockConfig;
    use crate::db::repo::Repo;
    use crate::db::stocks::NewWatch;
    use crate::stock::bars::{Bar, Series};
    use crate::stock::clock::market_day;
    use crate::stock::source::SourceError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A source that returns, for every symbol, a series whose session is
    /// *closed* at the fixed `now` it was built with. Counts calls.
    struct ClosedStub {
        now: i64,
        calls: AtomicUsize,
        down: bool,
    }

    impl ClosedStub {
        fn new(now: i64) -> Self {
            Self { now, calls: AtomicUsize::new(0), down: false }
        }
        fn down(now: i64) -> Self {
            Self { now, calls: AtomicUsize::new(0), down: true }
        }
    }

    fn closed_series(now: i64) -> Series {
        let day = market_day(now, 0);
        let start = day * 86_400;
        // 40 bars up to and including today, so indicators are defined.
        let bars: Vec<Bar> = (0..40)
            .map(|i| Bar {
                ts: start - (39 - i) * 86_400,
                open: Some(100.0 + i as f64),
                high: Some(101.0 + i as f64),
                low: Some(99.0 + i as f64),
                close: Some(100.0 + i as f64),
                volume: Some(1_000),
            })
            .collect();
        Series {
            bars,
            gmtoffset: 0,
            regular_start: start,
            regular_end: now - 600, // bell rang 10 min ago
            market_time: now,
            last_price: Some(139.0),
            prev_close: Some(138.0),
            week52_high: Some(200.0),
            week52_low: Some(50.0),
            exchange: "TAI".into(),
            display_name: "Stub".into(),
            currency: "TWD".into(),
        }
    }

    impl StockSource for ClosedStub {
        async fn series(&self, _sym: &Symbol, _days: u16) -> anyhow::Result<Series> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.down {
                Err(SourceError::Unreachable("down".into()).into())
            } else {
                Ok(closed_series(self.now))
            }
        }
        fn supports(&self, _b: super::super::symbol::Board) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "stub"
        }
    }

    async fn setup(now: i64, source: ClosedStub) -> (Arc<StockService<ClosedStub>>, Repo) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::connect(dir.path().join("w.db").to_str().unwrap()).await.unwrap();
        std::mem::forget(dir);
        let repo = Repo::new(db);
        let cfg = StockConfig {
            default_delay_minutes_tw: 0,
            default_delay_minutes_us: 0,
            ..StockConfig::default()
        };
        let svc = Arc::new(StockService::new(repo.clone(), source, None, cfg));
        let _ = now;
        (svc, repo)
    }

    fn watch(chat: i64, symbol: &'static str, market: &'static str) -> NewWatch<'static> {
        NewWatch { chat_id: chat, created_by: chat, symbol, market, exchange: "", display_name: "", currency: "" }
    }

    #[tokio::test]
    async fn no_chats_due_makes_zero_upstream_requests() {
        let now = now_unix();
        let (svc, _repo) = setup(now, ClosedStub::new(now)).await;
        let probe = svc.clone();
        let worker = StockWorker::new(svc, RecordingSender::default());
        worker.run_once(now).await.unwrap();
        assert_eq!(probe.source().calls.load(Ordering::Relaxed), 0, "no watchlist entries -> no HTTP");
    }

    #[tokio::test]
    async fn report_is_sent_once_and_a_second_pass_sends_nothing() {
        let now = now_unix();
        let (svc, repo) = setup(now, ClosedStub::new(now)).await;
        repo.insert_watch(&watch(100, "2330.TW", "tw")).await.unwrap();
        let worker = StockWorker::new(svc, RecordingSender::default());

        worker.run_once(now).await.unwrap();
        assert_eq!(worker.sender.sent.lock().unwrap().len(), 1, "one report on the first pass");

        worker.run_once(now).await.unwrap();
        assert_eq!(worker.sender.sent.lock().unwrap().len(), 1, "ledger blocks a second send");
    }

    #[tokio::test]
    async fn symbols_are_fetched_once_per_symbol_not_once_per_chat() {
        let now = now_unix();
        let (svc, repo) = setup(now, ClosedStub::new(now)).await;
        // Three chats, all watching the same symbol.
        for chat in [1, 2, 3] {
            repo.insert_watch(&watch(chat, "2330.TW", "tw")).await.unwrap();
        }
        let probe = svc.clone();
        let worker = StockWorker::new(svc, RecordingSender::default());
        worker.run_once(now).await.unwrap();
        let calls = probe.source().calls.load(Ordering::Relaxed);
        // probe (2330.TW) fetched once; the watched symbol is the same and is
        // already fresh in cache — so at most 2 upstream calls, not 3+.
        assert!(calls <= 2, "expected symbol fetched once, got {calls} calls");
        assert_eq!(worker.sender.sent.lock().unwrap().len(), 3, "each chat gets its report");
    }

    #[tokio::test]
    async fn forbidden_send_keeps_the_watchlist_but_stamps_the_day() {
        let now = now_unix();
        let (svc, repo) = setup(now, ClosedStub::new(now)).await;
        repo.insert_watch(&watch(100, "2330.TW", "tw")).await.unwrap();
        let sender = RecordingSender { forbidden_chat_ids: vec![100], ..Default::default() };
        let worker = StockWorker::new(svc, sender);
        worker.run_once(now).await.unwrap();
        // Watchlist intact.
        assert_eq!(worker.stock.repo().count_watch_for_chat(100).await.unwrap(), 1);
        // Day stamped forbidden (status 2) so we don't retry every tick.
        let status = worker.stock.repo().report_status(100, "tw", &market_date_string(market_day(now, 0))).await.unwrap();
        assert_eq!(status, Some(2));
    }

    #[tokio::test]
    async fn manual_stockreport_suppresses_that_evenings_automatic_push() {
        // A manual /stockreport claims + stamps the ledger for the trading day.
        // The worker's later pass must then find the day already handled and
        // send nothing — the two share the ledger.
        let now = now_unix();
        let (svc, repo) = setup(now, ClosedStub::new(now)).await;
        repo.insert_watch(&watch(100, "2330.TW", "tw")).await.unwrap();
        let trade_date = market_date_string(market_day(now, 0));
        // Stand in for the manual run: claim + mark sent.
        assert!(repo.claim_report(100, "tw", &trade_date, now, 1800, 3).await.unwrap());
        repo.mark_report_sent(100, "tw", &trade_date).await.unwrap();

        let worker = StockWorker::new(svc, RecordingSender::default());
        worker.run_once(now).await.unwrap();
        assert_eq!(worker.sender.sent.lock().unwrap().len(), 0, "auto push suppressed by the manual run");
    }

    #[tokio::test]
    async fn a_market_outage_is_swallowed_not_propagated() {
        // A down source for a market with due chats must not propagate out of
        // run_once (which would, via try_join!, take down the process) — the
        // per-market error is logged and the loop continues. This is what keeps
        // one market's outage from starving the other.
        let now = now_unix();
        let (svc, repo) = setup(now, ClosedStub::down(now)).await;
        repo.insert_watch(&watch(1, "AAPL", "us")).await.unwrap();
        repo.insert_watch(&watch(1, "2330.TW", "tw")).await.unwrap();
        let worker = StockWorker::new(svc, RecordingSender::default());
        assert!(worker.run_once(now).await.is_ok());
        assert_eq!(worker.sender.sent.lock().unwrap().len(), 0);
    }
}
