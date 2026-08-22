//! Repo methods for the stock-tracking feature (migration 0006). A second
//! `impl Repo` block, raw SQL with `?` binds, following the conventions in
//! `repo.rs` and `bookmarks.rs`.

use super::bookmarks::now_unix;
use super::models::{PushSetting, StockBar, StockMeta, WatchItem};
use super::repo::Repo;
use super::DbResult;

/// Column list matching `models::WatchItem`. Read paths bind positionally.
const WATCH_COLS: &str = "id, chat_id, created_by, symbol, market, exchange, \
     display_name, currency, note, sort_order, created_at, updated_at";

const META_COLS: &str = "symbol, market, exchange, display_name, currency, gmtoffset, \
     session_start, session_end, market_time, last_price, prev_close, week52_high, \
     week52_low, trade_date, source, fetched_at, updated_at";

const BAR_COLS: &str = "symbol, trade_date, ts, open, high, low, close, volume, source, updated_at";

const PUSH_COLS: &str = "chat_id, market, enabled, push_minute, updated_at";

/// Report ledger status codes (`stock_report_log.status`).
pub const REPORT_CLAIMED: i64 = 0;
pub const REPORT_SENT: i64 = 1;
pub const REPORT_FORBIDDEN: i64 = 2;

/// Fields for a new watchlist insert.
pub struct NewWatch<'a> {
    pub chat_id: i64,
    pub created_by: i64,
    pub symbol: &'a str,
    pub market: &'a str,
    pub exchange: &'a str,
    pub display_name: &'a str,
    pub currency: &'a str,
}

/// Outcome of [`Repo::insert_watch`].
pub struct WatchInsert {
    pub id: i64,
    /// True when the (chat_id, symbol) row already existed.
    pub existed: bool,
}

impl Repo {
    // --- watchlist ---

    /// Inserts a watched stock, or reports the existing (chat_id, symbol) row.
    ///
    /// A pre-check drives the `existed` flag rather than the `created_at != now`
    /// trick `upsert_bookmark` uses: two `/stockadd`s of the same symbol land in
    /// the same clock second, so that comparison would misreport a re-add as
    /// new. This is the (cold) add path, so a look-before-write is fine, and the
    /// unique index is the real guard against a racing duplicate.
    pub async fn insert_watch(&self, new: &NewWatch<'_>) -> DbResult<WatchInsert> {
        let now = now_unix();
        if let Some(id) = self
            .scalar_opt_i64(
                "SELECT id FROM stock_watchlist WHERE chat_id = ? AND symbol = ?",
                libsql::params![new.chat_id, new.symbol],
            )
            .await?
        {
            self.exec(
                "UPDATE stock_watchlist SET updated_at = ? WHERE id = ?",
                libsql::params![now, id],
            )
            .await?;
            return Ok(WatchInsert { id, existed: true });
        }
        let id = self
            .scalar_i64(
                "INSERT INTO stock_watchlist \
                 (chat_id, created_by, symbol, market, exchange, display_name, currency, \
                  note, sort_order, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, '', 0, ?, ?) \
                 RETURNING id",
                libsql::params![
                    new.chat_id,
                    new.created_by,
                    new.symbol,
                    new.market,
                    new.exchange,
                    new.display_name,
                    new.currency,
                    now,
                    now,
                ],
            )
            .await?;
        Ok(WatchInsert { id, existed: false })
    }

    pub async fn count_watch_for_chat(&self, chat_id: i64) -> DbResult<i64> {
        self.scalar_i64(
            "SELECT COUNT(*) FROM stock_watchlist WHERE chat_id = ?",
            libsql::params![chat_id],
        )
        .await
    }

    pub async fn count_distinct_symbols_global(&self) -> DbResult<i64> {
        self.scalar_i64("SELECT COUNT(DISTINCT symbol) FROM stock_watchlist", ())
            .await
    }

    /// Whether any chat tracks this symbol — lets the global-cap check allow a
    /// second chat to add an already-tracked symbol (distinct count won't grow).
    pub async fn symbol_tracked_anywhere(&self, symbol: &str) -> DbResult<bool> {
        Ok(self
            .scalar_i64(
                "SELECT COUNT(*) FROM stock_watchlist WHERE symbol = ?",
                libsql::params![symbol],
            )
            .await?
            > 0)
    }

    pub async fn get_watch_by_symbol(
        &self,
        chat_id: i64,
        symbol: &str,
    ) -> DbResult<Option<WatchItem>> {
        self.query_opt::<WatchItem>(
            &format!("SELECT {WATCH_COLS} FROM stock_watchlist WHERE chat_id = ? AND symbol = ?"),
            libsql::params![chat_id, symbol],
        )
        .await
    }

    pub async fn get_watch(&self, chat_id: i64, id: i64) -> DbResult<Option<WatchItem>> {
        self.query_opt::<WatchItem>(
            &format!("SELECT {WATCH_COLS} FROM stock_watchlist WHERE chat_id = ? AND id = ?"),
            libsql::params![chat_id, id],
        )
        .await
    }

    pub async fn delete_watch(&self, chat_id: i64, id: i64) -> DbResult<bool> {
        let affected = self
            .exec(
                "DELETE FROM stock_watchlist WHERE chat_id = ? AND id = ?",
                libsql::params![chat_id, id],
            )
            .await?;
        Ok(affected > 0)
    }

    pub async fn set_watch_note(&self, chat_id: i64, id: i64, note: &str) -> DbResult<bool> {
        let affected = self
            .exec(
                "UPDATE stock_watchlist SET note = ?, updated_at = ? WHERE chat_id = ? AND id = ?",
                libsql::params![note, now_unix(), chat_id, id],
            )
            .await?;
        Ok(affected > 0)
    }

    /// A page of a chat's watchlist. `market = None` means all markets.
    pub async fn list_watch(
        &self,
        chat_id: i64,
        market: Option<&str>,
        offset: i64,
        limit: i64,
    ) -> DbResult<Vec<WatchItem>> {
        match market {
            Some(m) => {
                self.query_all::<WatchItem>(
                    &format!(
                        "SELECT {WATCH_COLS} FROM stock_watchlist \
                         WHERE chat_id = ? AND market = ? \
                         ORDER BY sort_order, id LIMIT ? OFFSET ?"
                    ),
                    libsql::params![chat_id, m, limit, offset],
                )
                .await
            }
            None => {
                self.query_all::<WatchItem>(
                    &format!(
                        "SELECT {WATCH_COLS} FROM stock_watchlist \
                         WHERE chat_id = ? ORDER BY sort_order, id LIMIT ? OFFSET ?"
                    ),
                    libsql::params![chat_id, limit, offset],
                )
                .await
            }
        }
    }

    pub async fn count_watch(&self, chat_id: i64, market: Option<&str>) -> DbResult<i64> {
        match market {
            Some(m) => {
                self.scalar_i64(
                    "SELECT COUNT(*) FROM stock_watchlist WHERE chat_id = ? AND market = ?",
                    libsql::params![chat_id, m],
                )
                .await
            }
            None => {
                self.scalar_i64(
                    "SELECT COUNT(*) FROM stock_watchlist WHERE chat_id = ?",
                    libsql::params![chat_id],
                )
                .await
            }
        }
    }

    /// All watched symbols for a chat/market, for building its close report.
    pub async fn watch_for_chat_market(
        &self,
        chat_id: i64,
        market: &str,
    ) -> DbResult<Vec<WatchItem>> {
        self.query_all::<WatchItem>(
            &format!(
                "SELECT {WATCH_COLS} FROM stock_watchlist \
                 WHERE chat_id = ? AND market = ? ORDER BY sort_order, id"
            ),
            libsql::params![chat_id, market],
        )
        .await
    }

    /// The worker fetches each symbol once per pass: 100 chats tracking the same
    /// symbol collapse to one upstream request.
    pub async fn distinct_symbols_for_market(&self, market: &str) -> DbResult<Vec<String>> {
        self.scalar_all_string(
            "SELECT DISTINCT symbol FROM stock_watchlist WHERE market = ? ORDER BY symbol",
            libsql::params![market],
        )
        .await
    }

    /// Distinct chats that hold at least one symbol in `market`. These are the
    /// candidates for a close push; a chat with no `stock_push_settings` row is
    /// still included (enabled is the default).
    pub async fn chats_with_market(&self, market: &str) -> DbResult<Vec<i64>> {
        self.scalar_all_i64(
            "SELECT DISTINCT chat_id FROM stock_watchlist WHERE market = ? ORDER BY chat_id",
            libsql::params![market],
        )
        .await
    }

    // --- meta ---

    pub async fn upsert_meta(&self, m: &StockMeta) -> DbResult<()> {
        self.exec(
            &format!(
                "INSERT INTO stock_meta ({META_COLS}) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(symbol) DO UPDATE SET \
                   market=excluded.market, exchange=excluded.exchange, \
                   display_name=excluded.display_name, currency=excluded.currency, \
                   gmtoffset=excluded.gmtoffset, session_start=excluded.session_start, \
                   session_end=excluded.session_end, market_time=excluded.market_time, \
                   last_price=excluded.last_price, prev_close=excluded.prev_close, \
                   week52_high=excluded.week52_high, week52_low=excluded.week52_low, \
                   trade_date=excluded.trade_date, source=excluded.source, \
                   fetched_at=excluded.fetched_at, updated_at=excluded.updated_at"
            ),
            libsql::params![
                m.symbol.as_str(),
                m.market.as_str(),
                m.exchange.as_str(),
                m.display_name.as_str(),
                m.currency.as_str(),
                m.gmtoffset,
                m.session_start,
                m.session_end,
                m.market_time,
                m.last_price,
                m.prev_close,
                m.week52_high,
                m.week52_low,
                m.trade_date.as_str(),
                m.source.as_str(),
                m.fetched_at,
                m.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_meta(&self, symbol: &str) -> DbResult<Option<StockMeta>> {
        self.query_opt::<StockMeta>(
            &format!("SELECT {META_COLS} FROM stock_meta WHERE symbol = ?"),
            libsql::params![symbol],
        )
        .await
    }

    // --- bars ---

    /// Upserts a batch of bars in one transaction. PK (symbol, trade_date) makes
    /// each a natural upsert; re-fetching a day overwrites it in place.
    pub async fn upsert_bars(&self, bars: &[StockBar]) -> DbResult<()> {
        if bars.is_empty() {
            return Ok(());
        }
        let tx = self.conn().transaction().await?;
        for b in bars {
            tx.execute(
                "INSERT INTO stock_bars \
                 (symbol, trade_date, ts, open, high, low, close, volume, source, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(symbol, trade_date) DO UPDATE SET \
                   ts=excluded.ts, open=excluded.open, high=excluded.high, low=excluded.low, \
                   close=excluded.close, volume=excluded.volume, source=excluded.source, \
                   updated_at=excluded.updated_at",
                libsql::params![
                    b.symbol.as_str(),
                    b.trade_date.as_str(),
                    b.ts,
                    b.open,
                    b.high,
                    b.low,
                    b.close,
                    b.volume,
                    b.source.as_str(),
                    b.updated_at,
                ],
            )
            .await?;
        }
        tx.commit().await
    }

    /// The most recent `limit` bars, returned oldest-first (chronological) so
    /// the indicator functions can consume them directly.
    pub async fn recent_bars(&self, symbol: &str, limit: i64) -> DbResult<Vec<StockBar>> {
        let mut bars = self
            .query_all::<StockBar>(
                &format!(
                    "SELECT {BAR_COLS} FROM stock_bars WHERE symbol = ? ORDER BY ts DESC LIMIT ?"
                ),
                libsql::params![symbol, limit],
            )
            .await?;
        bars.reverse();
        Ok(bars)
    }

    /// Retention for one symbol's history. `keep_recent` **must** exceed the
    /// longest indicator window, or the first prune silently breaks long MAs.
    pub async fn prune_stock_bars(
        &self,
        symbol: &str,
        retention_days: u32,
        keep_recent: u32,
    ) -> DbResult<u64> {
        let modifier = format!("-{retention_days} days");
        self.exec(
            "DELETE FROM stock_bars \
             WHERE symbol = ? \
               AND trade_date < date('now', ?) \
               AND trade_date NOT IN ( \
                 SELECT trade_date FROM stock_bars WHERE symbol = ? ORDER BY ts DESC LIMIT ? \
               )",
            libsql::params![symbol, modifier, symbol, i64::from(keep_recent)],
        )
        .await
    }

    /// Drops bars and meta for symbols nobody tracks anymore. `prune_contents`
    /// has no equivalent — without this a symbol's whole history lingers forever
    /// after the last watcher removes it.
    pub async fn prune_orphan_symbols(&self) -> DbResult<u64> {
        let bars = self
            .exec(
                "DELETE FROM stock_bars \
                 WHERE symbol NOT IN (SELECT symbol FROM stock_watchlist)",
                (),
            )
            .await?;
        self.exec(
            "DELETE FROM stock_meta WHERE symbol NOT IN (SELECT symbol FROM stock_watchlist)",
            (),
        )
        .await?;
        Ok(bars)
    }

    // --- push settings ---

    pub async fn get_push_setting(
        &self,
        chat_id: i64,
        market: &str,
    ) -> DbResult<Option<PushSetting>> {
        self.query_opt::<PushSetting>(
            &format!(
                "SELECT {PUSH_COLS} FROM stock_push_settings WHERE chat_id = ? AND market = ?"
            ),
            libsql::params![chat_id, market],
        )
        .await
    }

    pub async fn set_push_setting(
        &self,
        chat_id: i64,
        market: &str,
        enabled: bool,
        push_minute: Option<i64>,
    ) -> DbResult<()> {
        self.exec(
            "INSERT INTO stock_push_settings (chat_id, market, enabled, push_minute, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(chat_id, market) DO UPDATE SET \
               enabled=excluded.enabled, push_minute=excluded.push_minute, \
               updated_at=excluded.updated_at",
            libsql::params![chat_id, market, i64::from(enabled), push_minute, now_unix()],
        )
        .await?;
        Ok(())
    }

    // --- report ledger ---

    /// Claims the right to send today's report for (chat, market, trade_date).
    ///
    /// A fresh insert claims it (`RETURNING` yields the row). On conflict the
    /// `DO UPDATE` fires only for a *stuck* claim — still `status=0`, older than
    /// `stale_secs`, under `max_attempts` — bumping attempts and re-claiming for
    /// a bounded retry. An already-sent (status=1) or forbidden (status=2) row,
    /// or a recent/exhausted claim, updates nothing and returns no row → skip.
    pub async fn claim_report(
        &self,
        chat_id: i64,
        market: &str,
        trade_date: &str,
        now: i64,
        stale_secs: i64,
        max_attempts: i64,
    ) -> DbResult<bool> {
        let row = self
            .scalar_opt_i64(
                "INSERT INTO stock_report_log (chat_id, market, trade_date, status, attempts, claimed_at) \
                 VALUES (?1, ?2, ?3, 0, 1, ?4) \
                 ON CONFLICT(chat_id, market, trade_date) DO UPDATE SET \
                   attempts = stock_report_log.attempts + 1, \
                   claimed_at = ?4 \
                 WHERE stock_report_log.status = 0 \
                   AND stock_report_log.claimed_at < ?4 - ?5 \
                   AND stock_report_log.attempts < ?6 \
                 RETURNING chat_id",
                libsql::params![chat_id, market, trade_date, now, stale_secs, max_attempts],
            )
            .await?;
        Ok(row.is_some())
    }

    pub async fn mark_report_sent(
        &self,
        chat_id: i64,
        market: &str,
        trade_date: &str,
    ) -> DbResult<()> {
        self.exec(
            "UPDATE stock_report_log SET status = 1, sent_at = ? \
             WHERE chat_id = ? AND market = ? AND trade_date = ?",
            libsql::params![now_unix(), chat_id, market, trade_date],
        )
        .await?;
        Ok(())
    }

    /// Forbidden still stamps the day (status=2): otherwise a blocked chat is
    /// retried every 60s forever, burning the global send budget.
    pub async fn mark_report_forbidden(
        &self,
        chat_id: i64,
        market: &str,
        trade_date: &str,
    ) -> DbResult<()> {
        self.exec(
            "UPDATE stock_report_log SET status = 2, sent_at = ? \
             WHERE chat_id = ? AND market = ? AND trade_date = ?",
            libsql::params![now_unix(), chat_id, market, trade_date],
        )
        .await?;
        Ok(())
    }

    /// `None` = never attempted; `Some(status)` otherwise. Lets `decide_push`
    /// treat an existing row as "already sent this trading day".
    pub async fn report_status(
        &self,
        chat_id: i64,
        market: &str,
        trade_date: &str,
    ) -> DbResult<Option<i64>> {
        self.scalar_opt_i64(
            "SELECT status FROM stock_report_log \
             WHERE chat_id = ? AND market = ? AND trade_date = ?",
            libsql::params![chat_id, market, trade_date],
        )
        .await
    }

    // --- commentary cache ---

    pub async fn get_commentary(
        &self,
        symbol: &str,
        trade_date: &str,
        lang: &str,
    ) -> DbResult<Option<String>> {
        self.scalar_opt_string(
            "SELECT body FROM stock_commentary WHERE symbol = ? AND trade_date = ? AND lang = ?",
            libsql::params![symbol, trade_date, lang],
        )
        .await
    }

    pub async fn put_commentary(
        &self,
        symbol: &str,
        trade_date: &str,
        lang: &str,
        body: &str,
        provider: &str,
    ) -> DbResult<()> {
        self.exec(
            "INSERT INTO stock_commentary (symbol, trade_date, lang, body, provider, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(symbol, trade_date, lang) DO UPDATE SET \
               body=excluded.body, provider=excluded.provider, created_at=excluded.created_at",
            libsql::params![symbol, trade_date, lang, body, provider, now_unix()],
        )
        .await?;
        Ok(())
    }

    /// Retention for the two `trade_date`-keyed tables (90 days by default).
    pub async fn prune_stock_logs(&self, retention_days: u32) -> DbResult<u64> {
        let modifier = format!("-{retention_days} days");
        let logs = self
            .exec(
                "DELETE FROM stock_report_log WHERE trade_date < date('now', ?)",
                libsql::params![modifier.as_str()],
            )
            .await?;
        self.exec(
            "DELETE FROM stock_commentary WHERE trade_date < date('now', ?)",
            libsql::params![modifier.as_str()],
        )
        .await?;
        Ok(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn test_repo() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        std::mem::forget(dir);
        Repo::new(db)
    }

    fn watch(chat_id: i64, symbol: &'static str, market: &'static str) -> NewWatch<'static> {
        NewWatch {
            chat_id,
            created_by: chat_id,
            symbol,
            market,
            exchange: "",
            display_name: "",
            currency: "",
        }
    }

    #[tokio::test]
    async fn insert_watch_dedups_on_chat_and_symbol() {
        let repo = test_repo().await;
        let first = repo.insert_watch(&watch(1, "2330.TW", "tw")).await.unwrap();
        assert!(!first.existed);
        let second = repo.insert_watch(&watch(1, "2330.TW", "tw")).await.unwrap();
        assert!(second.existed);
        assert_eq!(first.id, second.id);
        assert_eq!(repo.count_watch_for_chat(1).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn symbols_for_market_is_deduplicated_across_chats() {
        let repo = test_repo().await;
        repo.insert_watch(&watch(1, "2330.TW", "tw")).await.unwrap();
        repo.insert_watch(&watch(2, "2330.TW", "tw")).await.unwrap();
        repo.insert_watch(&watch(3, "2330.TW", "tw")).await.unwrap();
        repo.insert_watch(&watch(1, "AAPL", "us")).await.unwrap();
        let tw = repo.distinct_symbols_for_market("tw").await.unwrap();
        assert_eq!(tw, vec!["2330.TW".to_string()]);
        assert_eq!(repo.count_distinct_symbols_global().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn chats_due_for_push_includes_chats_with_no_settings_row() {
        let repo = test_repo().await;
        repo.insert_watch(&watch(10, "2330.TW", "tw")).await.unwrap();
        repo.insert_watch(&watch(20, "2330.TW", "tw")).await.unwrap();
        // Chat 10 has never touched settings, chat 20 turned it on explicitly.
        repo.set_push_setting(20, "tw", true, Some(900)).await.unwrap();
        let chats = repo.chats_with_market("tw").await.unwrap();
        assert_eq!(chats, vec![10, 20]);
        assert!(repo.get_push_setting(10, "tw").await.unwrap().is_none());
        let s20 = repo.get_push_setting(20, "tw").await.unwrap().unwrap();
        assert_eq!(s20.push_minute, Some(900));
    }

    #[tokio::test]
    async fn claim_report_returns_the_row_once_then_never_again() {
        let repo = test_repo().await;
        let now = 1_000_000;
        assert!(repo.claim_report(1, "tw", "2026-08-21", now, 1800, 3).await.unwrap());
        // Immediate re-claim: not stale, so denied.
        assert!(!repo.claim_report(1, "tw", "2026-08-21", now, 1800, 3).await.unwrap());
        repo.mark_report_sent(1, "tw", "2026-08-21").await.unwrap();
        // Even much later, a sent row is never re-claimed.
        assert!(!repo.claim_report(1, "tw", "2026-08-21", now + 100_000, 1800, 3).await.unwrap());
        assert_eq!(repo.report_status(1, "tw", "2026-08-21").await.unwrap(), Some(REPORT_SENT));
    }

    #[tokio::test]
    async fn a_stale_unsent_claim_is_retried_at_most_three_times() {
        let repo = test_repo().await;
        let mut now = 1_000_000;
        // Initial claim (attempts = 1).
        assert!(repo.claim_report(1, "tw", "d", now, 1800, 3).await.unwrap());
        let mut granted = 1;
        // Each subsequent pass is well past the stale window but never sends.
        for _ in 0..5 {
            now += 3600;
            if repo.claim_report(1, "tw", "d", now, 1800, 3).await.unwrap() {
                granted += 1;
            }
        }
        // 1 insert + 2 retries (at attempts 1 and 2) = 3 total; the pass at
        // attempts == 3 is refused.
        assert_eq!(granted, 3);
    }

    #[tokio::test]
    async fn forbidden_still_stamps_the_day_and_blocks_reclaim() {
        let repo = test_repo().await;
        let now = 1_000_000;
        assert!(repo.claim_report(1, "tw", "d", now, 1800, 3).await.unwrap());
        repo.mark_report_forbidden(1, "tw", "d").await.unwrap();
        assert_eq!(repo.report_status(1, "tw", "d").await.unwrap(), Some(REPORT_FORBIDDEN));
        assert!(!repo.claim_report(1, "tw", "d", now + 100_000, 1800, 3).await.unwrap());
    }

    #[tokio::test]
    async fn prune_keeps_more_bars_than_the_longest_indicator_window() {
        let repo = test_repo().await;
        // 300 old bars; keep_recent = 260 (~1 trading year), well above the
        // 60-day MA / 35-bar MACD warm-up.
        let bars: Vec<StockBar> = (0..300)
            .map(|i| StockBar {
                symbol: "2330.TW".into(),
                // Distinct, old (1970-*), lexically increasing dates.
                trade_date: chrono::DateTime::from_timestamp(i64::from(i) * 86_400, 0)
                    .unwrap()
                    .format("%Y-%m-%d")
                    .to_string(),
                ts: i64::from(i) * 86_400,
                open: Some(1.0),
                high: Some(1.0),
                low: Some(1.0),
                close: Some(1.0),
                volume: Some(1),
                source: "yahoo".into(),
                updated_at: 0,
            })
            .collect();
        repo.upsert_bars(&bars).await.unwrap();
        let deleted = repo.prune_stock_bars("2330.TW", 1, 260).await.unwrap();
        assert_eq!(deleted, 40);
        let kept = repo.recent_bars("2330.TW", 1000).await.unwrap();
        assert_eq!(kept.len(), 260);
        assert!(kept.len() > crate::stock::MIN_BARS_FOR_INDICATORS);
        // recent_bars returns oldest-first.
        assert!(kept[0].ts < kept[kept.len() - 1].ts);
    }

    #[tokio::test]
    async fn prune_orphan_symbols_drops_untracked_history() {
        let repo = test_repo().await;
        repo.insert_watch(&watch(1, "2330.TW", "tw")).await.unwrap();
        let bar = |sym: &str| StockBar {
            symbol: sym.into(),
            trade_date: "2026-08-21".into(),
            ts: 1,
            open: None,
            high: None,
            low: None,
            close: Some(1.0),
            volume: Some(1),
            source: "yahoo".into(),
            updated_at: 0,
        };
        repo.upsert_bars(&[bar("2330.TW"), bar("9999.TW")]).await.unwrap();
        repo.prune_orphan_symbols().await.unwrap();
        assert_eq!(repo.recent_bars("2330.TW", 10).await.unwrap().len(), 1);
        assert!(repo.recent_bars("9999.TW", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_and_count_respect_the_market_scope() {
        let repo = test_repo().await;
        repo.insert_watch(&watch(1, "2330.TW", "tw")).await.unwrap();
        repo.insert_watch(&watch(1, "6488.TWO", "tw")).await.unwrap();
        repo.insert_watch(&watch(1, "AAPL", "us")).await.unwrap();
        assert_eq!(repo.count_watch(1, None).await.unwrap(), 3);
        assert_eq!(repo.count_watch(1, Some("tw")).await.unwrap(), 2);
        let us = repo.list_watch(1, Some("us"), 0, 10).await.unwrap();
        assert_eq!(us.len(), 1);
        assert_eq!(us[0].symbol, "AAPL");
    }
}
