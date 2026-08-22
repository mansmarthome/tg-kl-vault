//! `StockService` — the internal service layer the user calls "the api". Every
//! Telegram handler and the schedule worker go through this one facade: it owns
//! cache read-through, source fallback, symbol resolution (the two-stage TWSE/
//! TPEx probe), watchlist CRUD, push settings, and the rate-limiter / 429
//! cooldown / 4xx hard-lock around every upstream call. Handlers stay thin
//! (parse → service → render → send); the worker calls the same methods.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tracing::error;

use crate::config::StockConfig;
use crate::db::bookmarks::now_unix;
use crate::db::models::{PushSetting, StockBar, StockMeta, WatchItem};
use crate::db::repo::Repo;
use crate::db::stocks::NewWatch;
use crate::ratelimit::MinIntervalLimiter;

use super::bars::{Bar, Series};
use super::clock::{market_date_string, market_day, SessionMeta};
use super::indicators::{self, Snapshot};
use super::render::ReportEntry;
use super::signals::{self, Signal};
use super::source::{classify_source_error, SourceError, StockSource, TwBar, TwOfficialSource};
use super::symbol::{self, Board, Market, Parsed, Symbol};

/// Politeness gate for upstream calls. The worker paces its own batch loop on
/// top of this; interactive calls just want low latency.
const SOURCE_MIN_INTERVAL: Duration = Duration::from_millis(300);
/// Bounded retries (stale-claim recovery) live in the ledger; here we cap the
/// escalating 429 cooldown ladder at 60 minutes.
const COOLDOWN_LADDER_MINS: [u64; 3] = [15, 30, 60];

/// Which market(s) a listing covers. Wire form `"a"|"tw"|"us"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketScope {
    All,
    Tw,
    Us,
}

impl MarketScope {
    pub fn market(self) -> Option<&'static str> {
        match self {
            MarketScope::All => None,
            MarketScope::Tw => Some("tw"),
            MarketScope::Us => Some("us"),
        }
    }

    pub fn as_wire(self) -> &'static str {
        match self {
            MarketScope::All => "a",
            MarketScope::Tw => "tw",
            MarketScope::Us => "us",
        }
    }

    pub fn from_wire(s: &str) -> Option<MarketScope> {
        match s {
            "a" => Some(MarketScope::All),
            "tw" => Some(MarketScope::Tw),
            "us" => Some(MarketScope::Us),
            _ => None,
        }
    }
}

/// A rendered-ready view of one symbol: cached meta + freshly computed
/// indicators and signals. `stale` means the upstream refresh failed and this is
/// last-known data (rendered with a ⚠️ marker), not an error.
#[derive(Debug, Clone)]
pub struct QuoteView {
    pub symbol: Symbol,
    pub meta: Option<StockMeta>,
    pub snapshot: Snapshot,
    pub signals: Vec<Signal>,
    pub stale: bool,
}

/// Outcome of a successful [`StockService::add`].
#[derive(Debug, Clone)]
pub struct AddOutcome {
    pub id: i64,
    pub symbol: Symbol,
    pub display_name: String,
    /// True when the chat already tracked this symbol.
    pub existed: bool,
}

/// Typed add failures — the callers (`/stockadd`, the 🔖 button) render four
/// distinct zh-TW messages, so this is an enum, not `anyhow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    NotFound,
    LimitReachedChat(u32),
    LimitReachedGlobal(u32),
    /// Upstream unavailable or an internal (DB) error — "try again later".
    Upstream,
}

/// A page of a chat's watchlist, with the clamped page index resolved so the
/// bot layer only has to render.
#[derive(Debug, Clone)]
pub struct WatchlistPage {
    pub items: Vec<WatchItem>,
    pub total: usize,
    pub page_index: usize,
    pub per_page: usize,
    pub scope: MarketScope,
}

pub struct StockService<S: StockSource> {
    repo: Repo,
    source: S,
    tw_fallback: Option<TwOfficialSource>,
    config: StockConfig,
    limiter: MinIntervalLimiter,
    disabled: AtomicBool,
    consecutive_429: AtomicU32,
    cooldown_until: Mutex<Option<Instant>>,
}

impl<S: StockSource> StockService<S> {
    pub fn new(
        repo: Repo,
        source: S,
        tw_fallback: Option<TwOfficialSource>,
        config: StockConfig,
    ) -> Self {
        Self {
            repo,
            source,
            tw_fallback,
            config,
            limiter: MinIntervalLimiter::new(SOURCE_MIN_INTERVAL),
            disabled: AtomicBool::new(false),
            consecutive_429: AtomicU32::new(0),
            cooldown_until: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &StockConfig {
        &self.config
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn tw_fallback(&self) -> Option<&TwOfficialSource> {
        self.tw_fallback.as_ref()
    }

    // --- upstream call guarding: rate limit + 429 cooldown + 4xx hard lock ---

    /// The one path to the primary source. Short-circuits while hard-locked
    /// (a prior 401/403) or in a 429 cooldown, otherwise paces via the limiter
    /// and records the outcome so the next call sees the updated state.
    pub(crate) async fn guarded_series(&self, sym: &Symbol) -> anyhow::Result<Series> {
        if self.disabled.load(Ordering::Relaxed) {
            return Err(SourceError::Http(403).into());
        }
        if self.in_cooldown() {
            return Err(SourceError::RateLimited.into());
        }
        self.limiter.until_ready().await;
        let result = self.source.series(sym, self.config.history_days).await;
        self.note_result(&result);
        result
    }

    fn in_cooldown(&self) -> bool {
        let cd = self.cooldown_until.lock().unwrap();
        cd.is_some_and(|until| Instant::now() < until)
    }

    fn note_result<T>(&self, result: &anyhow::Result<T>) {
        match result {
            Ok(_) => {
                self.consecutive_429.store(0, Ordering::Relaxed);
                *self.cooldown_until.lock().unwrap() = None;
            }
            Err(err) => match classify_source_error(err) {
                SourceError::RateLimited => {
                    let n = self.consecutive_429.fetch_add(1, Ordering::Relaxed) as usize;
                    let mins = COOLDOWN_LADDER_MINS[n.min(COOLDOWN_LADDER_MINS.len() - 1)];
                    *self.cooldown_until.lock().unwrap() =
                        Some(Instant::now() + Duration::from_secs(mins * 60));
                }
                SourceError::Http(401 | 403) => {
                    let was_disabled = self.disabled.swap(true, Ordering::Relaxed);
                    if !was_disabled {
                        error!(source = self.source.name(), "stock source hard-locked (401/403)");
                    }
                }
                _ => {}
            },
        }
    }

    // --- resolution (add-time only; never on the hot path) ---

    /// Resolves a raw user string to a confirmed canonical `Symbol`, performing
    /// the two-stage TWSE→TPEx probe for a bare Taiwan code. Cache-first: a
    /// symbol already in `stock_meta` resolves with zero requests.
    pub async fn resolve(&self, raw: &str) -> Result<Symbol, SourceError> {
        match symbol::parse(raw) {
            Parsed::Invalid(_) => Err(SourceError::NotFound),
            Parsed::Resolved(sym) => {
                if self.cached_meta(&sym.canonical).await.is_some() {
                    return Ok(sym);
                }
                match self.guarded_series(&sym).await {
                    Ok(series) => {
                        let _ = self.cache_series(&sym, &series).await;
                        Ok(sym)
                    }
                    Err(err) => Err(classify_source_error(&err)),
                }
            }
            Parsed::TaiwanAmbiguous { local_code } => self.resolve_taiwan(&local_code).await,
        }
    }

    async fn resolve_taiwan(&self, local_code: &str) -> Result<Symbol, SourceError> {
        // Cache hit for either board -> zero requests.
        for board in [Board::Twse, Board::Tpex] {
            let cand = tw_candidate(local_code, board);
            if self.cached_meta(&cand.canonical).await.is_some() {
                return Ok(cand);
            }
        }
        // Probe .TW (expect exchange TAI) then .TWO (expect TWO). A 404 means
        // "not this board", so continue; any other error is a real upstream
        // problem and must not be misreported as "symbol not found".
        for (board, want_exchange) in [(Board::Twse, "TAI"), (Board::Tpex, "TWO")] {
            let cand = tw_candidate(local_code, board);
            match self.guarded_series(&cand).await {
                Ok(series) => {
                    if series.exchange.is_empty() || series.exchange == want_exchange {
                        let _ = self.cache_series(&cand, &series).await;
                        return Ok(cand);
                    }
                }
                Err(err) => match classify_source_error(&err) {
                    SourceError::NotFound => continue,
                    other => return Err(other),
                },
            }
        }
        Err(SourceError::NotFound)
    }

    // --- cache read-through ---

    async fn cached_meta(&self, canonical: &str) -> Option<StockMeta> {
        self.repo.get_meta(canonical).await.ok().flatten()
    }

    fn is_fresh(&self, meta: Option<&StockMeta>, now: i64) -> bool {
        meta.is_some_and(|m| now - m.fetched_at < self.config.cache_ttl_seconds as i64)
    }

    /// Refreshes one symbol from the primary source and writes it to the cache.
    pub(crate) async fn refresh(&self, sym: &Symbol) -> anyhow::Result<()> {
        let series = self.guarded_series(sym).await?;
        self.cache_series(sym, &series).await
    }

    /// Sanitizes and persists a fetched series' bars + meta. Non-sane bars
    /// (null close, bad ranges) are dropped here, never zero-filled.
    pub(crate) async fn cache_series(&self, sym: &Symbol, series: &Series) -> anyhow::Result<()> {
        let now = now_unix();
        let source = self.source.name();
        let bars: Vec<StockBar> = series
            .bars
            .iter()
            .filter(|b| super::bars::bar_is_sane(b, 0, now))
            .map(|b| StockBar {
                symbol: sym.canonical.clone(),
                trade_date: market_date_string(market_day(b.ts, series.gmtoffset)),
                ts: b.ts,
                open: b.open,
                high: b.high,
                low: b.low,
                close: b.close,
                volume: b.volume,
                source: source.to_owned(),
                updated_at: now,
            })
            .collect();
        self.repo.upsert_bars(&bars).await?;

        let trade_date = bars.last().map(|b| b.trade_date.clone()).unwrap_or_default();
        let meta = StockMeta {
            symbol: sym.canonical.clone(),
            market: sym.market.as_wire().to_owned(),
            exchange: series.exchange.clone(),
            display_name: series.display_name.clone(),
            currency: series.currency.clone(),
            gmtoffset: series.gmtoffset,
            session_start: Some(series.regular_start),
            session_end: Some(series.regular_end),
            market_time: Some(series.market_time),
            last_price: series.last_price,
            prev_close: series.prev_close,
            week52_high: series.week52_high,
            week52_low: series.week52_low,
            trade_date,
            source: source.to_owned(),
            fetched_at: now,
            updated_at: now,
        };
        self.repo.upsert_meta(&meta).await?;
        Ok(())
    }

    /// Cache-first quote + indicators. On upstream failure with cached data,
    /// returns the last-known view flagged `stale` rather than erroring.
    pub async fn snapshot(&self, sym: &Symbol, now: i64) -> anyhow::Result<QuoteView> {
        let meta = self.cached_meta(&sym.canonical).await;
        let mut stale = false;
        if !self.is_fresh(meta.as_ref(), now) {
            if let Err(err) = self.refresh(sym).await {
                let have_cache =
                    meta.is_some() || !self.repo.recent_bars(&sym.canonical, 1).await?.is_empty();
                if !have_cache {
                    return Err(err);
                }
                stale = true;
            }
        }
        let meta = self.cached_meta(&sym.canonical).await;
        let bars = self.load_bars(&sym.canonical).await?;
        Ok(QuoteView {
            symbol: sym.clone(),
            meta,
            snapshot: indicators::snapshot(&bars),
            signals: signals::detect(&bars),
            stale,
        })
    }

    /// Cache-first history for the indicator/chart views.
    pub async fn history(&self, sym: &Symbol, days: u16, now: i64) -> anyhow::Result<Vec<Bar>> {
        let meta = self.cached_meta(&sym.canonical).await;
        if !self.is_fresh(meta.as_ref(), now) {
            let _ = self.refresh(sym).await; // best-effort; fall back to cache
        }
        let bars = self.repo.recent_bars(&sym.canonical, i64::from(days)).await?;
        Ok(bars.iter().map(to_bar).collect())
    }

    /// Fetches the probe/symbol's current session clock, caching the series as a
    /// side effect. Used by the worker to classify the market state each pass.
    pub async fn fetch_session_meta(&self, sym: &Symbol) -> anyhow::Result<SessionMeta> {
        let series = self.guarded_series(sym).await?;
        let meta = series.session_meta();
        self.cache_series(sym, &series).await?;
        Ok(meta)
    }

    /// Builds one report entry per watched symbol for a chat/market. Each symbol
    /// is served cache-first via `snapshot`; a TW symbol with no usable Yahoo
    /// data falls back to the official dump **only when the dump's date matches**
    /// `trade_date`, with the indicator block suppressed (a partial history would
    /// produce confidently-wrong averages).
    pub async fn report_entries(
        &self,
        chat_id: i64,
        market: Market,
        trade_date: &str,
        now: i64,
    ) -> anyhow::Result<Vec<ReportEntry>> {
        let items = self.repo.watch_for_chat_market(chat_id, market.as_wire()).await?;
        let mut out = Vec::new();
        for item in items {
            let Parsed::Resolved(sym) = symbol::parse(&item.symbol) else {
                continue;
            };
            match self.snapshot(&sym, now).await {
                Ok(view) if view.snapshot.last_close.is_some() => {
                    out.push(entry_from_view(&sym, &item, view));
                    continue;
                }
                _ => {}
            }
            if market == Market::Tw {
                if let Some(fb) = &self.tw_fallback {
                    if let Ok(Some(bar)) = fb.fallback_quote(&sym, trade_date).await {
                        out.push(fallback_entry(&item, &sym, &bar));
                    }
                }
            }
        }
        Ok(out)
    }

    pub(crate) async fn load_bars(&self, canonical: &str) -> anyhow::Result<Vec<Bar>> {
        let bars = self
            .repo
            .recent_bars(canonical, i64::from(self.config.history_days))
            .await?;
        Ok(bars.iter().map(to_bar).collect())
    }

    // --- watchlist CRUD ---

    pub async fn add(
        &self,
        chat_id: i64,
        created_by: i64,
        raw: &str,
    ) -> Result<AddOutcome, AddError> {
        // Per-chat cap first, so a full chat never spends an upstream probe.
        let per_chat = self
            .repo
            .count_watch_for_chat(chat_id)
            .await
            .map_err(|_| AddError::Upstream)?;
        if per_chat >= i64::from(self.config.max_symbols_per_chat) {
            return Err(AddError::LimitReachedChat(self.config.max_symbols_per_chat));
        }

        let sym = self.resolve(raw).await.map_err(|e| match e {
            SourceError::NotFound => AddError::NotFound,
            _ => AddError::Upstream,
        })?;

        // Global distinct-symbol cap: only blocks a *new* symbol.
        let global = self
            .repo
            .count_distinct_symbols_global()
            .await
            .map_err(|_| AddError::Upstream)?;
        if global >= i64::from(self.config.max_symbols_global)
            && !self
                .repo
                .symbol_tracked_anywhere(&sym.canonical)
                .await
                .map_err(|_| AddError::Upstream)?
        {
            return Err(AddError::LimitReachedGlobal(self.config.max_symbols_global));
        }

        let meta = self.cached_meta(&sym.canonical).await;
        let (display_name, currency, exchange) = meta
            .map(|m| (m.display_name, m.currency, m.exchange))
            .unwrap_or_default();
        let ins = self
            .repo
            .insert_watch(&NewWatch {
                chat_id,
                created_by,
                symbol: &sym.canonical,
                market: sym.market.as_wire(),
                exchange: &exchange,
                display_name: &display_name,
                currency: &currency,
            })
            .await
            .map_err(|_| AddError::Upstream)?;

        Ok(AddOutcome {
            id: ins.id,
            symbol: sym,
            display_name,
            existed: ins.existed,
        })
    }

    pub async fn remove(&self, chat_id: i64, id: i64) -> anyhow::Result<bool> {
        Ok(self.repo.delete_watch(chat_id, id).await?)
    }

    pub async fn get_watch(&self, chat_id: i64, id: i64) -> anyhow::Result<Option<WatchItem>> {
        Ok(self.repo.get_watch(chat_id, id).await?)
    }

    pub async fn get_watch_by_symbol(
        &self,
        chat_id: i64,
        symbol: &str,
    ) -> anyhow::Result<Option<WatchItem>> {
        Ok(self.repo.get_watch_by_symbol(chat_id, symbol).await?)
    }

    pub async fn set_note(&self, chat_id: i64, id: i64, note: &str) -> anyhow::Result<bool> {
        Ok(self.repo.set_watch_note(chat_id, id, note).await?)
    }

    pub async fn list_page(
        &self,
        chat_id: i64,
        scope: MarketScope,
        page: usize,
    ) -> anyhow::Result<WatchlistPage> {
        let per_page = self.config.watchlist_page_size.max(1) as usize;
        let total = self.repo.count_watch(chat_id, scope.market()).await? as usize;
        let pages = total.div_ceil(per_page).max(1);
        let page_index = page.min(pages - 1);
        let items = self
            .repo
            .list_watch(
                chat_id,
                scope.market(),
                (page_index * per_page) as i64,
                per_page as i64,
            )
            .await?;
        Ok(WatchlistPage {
            items,
            total,
            page_index,
            per_page,
            scope,
        })
    }

    // --- push settings ---

    /// Both markets' settings, filling defaults (enabled, no explicit time)
    /// where no row exists.
    pub async fn push_settings(&self, chat_id: i64) -> anyhow::Result<[PushSetting; 2]> {
        let fill = |market: &str, row: Option<PushSetting>| {
            row.unwrap_or(PushSetting {
                chat_id,
                market: market.to_owned(),
                enabled: 1,
                push_minute: None,
                updated_at: 0,
            })
        };
        let tw = self.repo.get_push_setting(chat_id, "tw").await?;
        let us = self.repo.get_push_setting(chat_id, "us").await?;
        Ok([fill("tw", tw), fill("us", us)])
    }

    pub async fn set_push(
        &self,
        chat_id: i64,
        market: Market,
        enabled: bool,
        minute: Option<i64>,
    ) -> anyhow::Result<()> {
        self.repo
            .set_push_setting(chat_id, market.as_wire(), enabled, minute)
            .await?;
        Ok(())
    }
}

fn tw_candidate(local_code: &str, board: Board) -> Symbol {
    Symbol {
        canonical: format!("{local_code}{}", board.yahoo_suffix()),
        market: Market::Tw,
        board,
        local_code: local_code.to_owned(),
    }
}

fn display_name_for<'a>(item: &'a WatchItem, meta: Option<&'a StockMeta>) -> String {
    if !item.display_name.is_empty() {
        return item.display_name.clone();
    }
    meta.map(|m| m.display_name.clone()).unwrap_or_default()
}

fn entry_from_view(sym: &Symbol, item: &WatchItem, view: QuoteView) -> ReportEntry {
    let (w52h, w52l) = view
        .meta
        .as_ref()
        .map(|m| (m.week52_high, m.week52_low))
        .unwrap_or((None, None));
    ReportEntry {
        canonical: sym.canonical.clone(),
        local_code: sym.local_code.clone(),
        display_name: display_name_for(item, view.meta.as_ref()),
        snapshot: view.snapshot,
        signals: view.signals,
        week52_high: w52h,
        week52_low: w52l,
        indicators_unavailable: false,
        commentary: None,
    }
}

fn fallback_entry(item: &WatchItem, sym: &Symbol, bar: &TwBar) -> ReportEntry {
    let prev_close = match (bar.close, bar.change) {
        (Some(c), Some(ch)) => Some(c - ch),
        _ => None,
    };
    let snapshot = Snapshot {
        bars_used: 1,
        last_close: bar.close,
        prev_close,
        last_volume: bar.volume,
        ..Snapshot::default()
    };
    ReportEntry {
        canonical: sym.canonical.clone(),
        local_code: sym.local_code.clone(),
        display_name: display_name_for(item, None),
        snapshot,
        signals: Vec::new(),
        week52_high: None,
        week52_low: None,
        // Only a single fallback close: never render a partial indicator block.
        indicators_unavailable: true,
        commentary: None,
    }
}

/// Converts a stored bar row into the pure `Bar` the indicators consume.
pub(crate) fn to_bar(b: &StockBar) -> Bar {
    Bar {
        ts: b.ts,
        open: b.open,
        high: b.high,
        low: b.low,
        close: b.close,
        volume: b.volume,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::sync::atomic::AtomicUsize;

    /// A `StockSource` stub: counts calls and returns a fixed series or errors.
    struct StubSource {
        calls: AtomicUsize,
        ok: bool,
        exchange: &'static str,
    }

    impl StubSource {
        fn ok(exchange: &'static str) -> Self {
            Self { calls: AtomicUsize::new(0), ok: true, exchange }
        }
        fn down() -> Self {
            Self { calls: AtomicUsize::new(0), ok: false, exchange: "" }
        }
        fn count(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    fn stub_series(exchange: &str) -> Series {
        // 40 bars so indicators are defined.
        let bars = (0..40)
            .map(|i| Bar {
                ts: 1_700_000_000 + i * 86_400,
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
            regular_start: 1,
            regular_end: 2,
            market_time: 3,
            last_price: Some(140.0),
            prev_close: Some(139.0),
            week52_high: Some(200.0),
            week52_low: Some(50.0),
            exchange: exchange.to_owned(),
            display_name: "Stub Co".to_owned(),
            currency: "USD".to_owned(),
        }
    }

    impl StockSource for StubSource {
        async fn series(&self, _sym: &Symbol, _days: u16) -> anyhow::Result<Series> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.ok {
                Ok(stub_series(self.exchange))
            } else {
                Err(SourceError::Unreachable("down".into()).into())
            }
        }
        fn supports(&self, _board: Board) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "stub"
        }
    }

    async fn service(source: StubSource) -> StockService<StubSource> {
        let dir = tempfile::tempdir().unwrap();
        let db = db::connect(dir.path().join("s.db").to_str().unwrap()).await.unwrap();
        std::mem::forget(dir);
        let cfg = StockConfig {
            max_symbols_per_chat: 50,
            ..StockConfig::default()
        };
        StockService::new(Repo::new(db), source, None, cfg)
    }

    fn aapl() -> Symbol {
        match symbol::parse("AAPL") {
            Parsed::Resolved(s) => s,
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn resolve_uses_the_cache_and_makes_zero_requests_on_repeat() {
        let svc = service(StubSource::ok("NMS")).await;
        let first = svc.resolve("AAPL").await.unwrap();
        assert_eq!(first.canonical, "AAPL");
        assert_eq!(svc.source().count(), 1, "first resolve probes once");
        svc.resolve("AAPL").await.unwrap();
        assert_eq!(svc.source().count(), 1, "second resolve is a cache hit");
    }

    #[tokio::test]
    async fn total_failure_with_a_cache_row_returns_stale_not_error() {
        // Warm the cache with a healthy source, then swap in a dead one.
        let svc = service(StubSource::ok("NMS")).await;
        svc.snapshot(&aapl(), 1_000_000_000).await.unwrap();
        let dead = service(StubSource::down()).await;
        // Seed the dead service's DB by hand via cache_series.
        dead.cache_series(&aapl(), &stub_series("NMS")).await.unwrap();
        // now far in the future so the cache is stale and a refresh is attempted.
        let view = dead.snapshot(&aapl(), 9_999_999_999).await.unwrap();
        assert!(view.stale, "a failed refresh over cached data must be stale, not an error");
        assert!(view.snapshot.last_close.is_some());
    }

    #[tokio::test]
    async fn snapshot_without_any_cache_propagates_the_error() {
        let svc = service(StubSource::down()).await;
        assert!(svc.snapshot(&aapl(), 1_000_000_000).await.is_err());
    }

    #[tokio::test]
    async fn add_beyond_the_per_chat_cap_is_a_typed_error_not_anyhow() {
        let svc = service(StubSource::ok("NMS")).await;
        {
            let cfg = StockConfig {
                max_symbols_per_chat: 1,
                ..StockConfig::default()
            };
            // rebuild with cap 1
            let dir = tempfile::tempdir().unwrap();
            let db = db::connect(dir.path().join("s.db").to_str().unwrap()).await.unwrap();
            std::mem::forget(dir);
            let svc = StockService::new(Repo::new(db), StubSource::ok("NMS"), None, cfg);
            let first = svc.add(1, 1, "AAPL").await.unwrap();
            assert!(!first.existed);
            let err = svc.add(1, 1, "MSFT").await.unwrap_err();
            assert_eq!(err, AddError::LimitReachedChat(1));
        }
        // A bad symbol is a typed NotFound.
        let bad = svc.add(1, 1, "!!!bad!!!").await.unwrap_err();
        assert_eq!(bad, AddError::NotFound);
    }

    #[tokio::test]
    async fn add_is_idempotent_and_reports_existed() {
        let svc = service(StubSource::ok("NMS")).await;
        assert!(!svc.add(7, 7, "AAPL").await.unwrap().existed);
        let again = svc.add(7, 7, "AAPL").await.unwrap();
        assert!(again.existed);
    }

    #[tokio::test]
    async fn list_page_clamps_a_stale_page() {
        let svc = service(StubSource::ok("NMS")).await;
        svc.add(1, 1, "AAPL").await.unwrap();
        let page = svc.list_page(1, MarketScope::All, 99).await.unwrap();
        assert_eq!(page.page_index, 0);
        assert_eq!(page.total, 1);
        assert_eq!(page.items.len(), 1);
    }

    #[tokio::test]
    async fn push_settings_default_to_enabled_when_absent() {
        let svc = service(StubSource::ok("NMS")).await;
        let [tw, us] = svc.push_settings(42).await.unwrap();
        assert_eq!(tw.enabled, 1);
        assert_eq!(us.enabled, 1);
        assert_eq!(tw.push_minute, None);
        svc.set_push(42, Market::Tw, false, Some(840)).await.unwrap();
        let [tw, _] = svc.push_settings(42).await.unwrap();
        assert_eq!(tw.enabled, 0);
        assert_eq!(tw.push_minute, Some(840));
    }

    #[test]
    fn market_scope_round_trips() {
        for s in [MarketScope::All, MarketScope::Tw, MarketScope::Us] {
            assert_eq!(MarketScope::from_wire(s.as_wire()), Some(s));
        }
    }
}
