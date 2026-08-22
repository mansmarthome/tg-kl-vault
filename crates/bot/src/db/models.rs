use super::FromRow;
use libsql::Row;

/// GORM stored timestamps as SQLite datetime strings. Keep them as strings in
/// phase 1 until we verify a real production data.db sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl FromRow for User {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            created_at: row.get(1)?,
            updated_at: row.get(2)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub id: i64,
    pub link: Option<String>,
    pub title: Option<String>,
    pub error_count: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub next_fetch_at: i64,
    /// Health history (migration 0005). `error_count` alone only says "it is
    /// broken"; these say *how* and *since when*, and are what `/feedcheck`
    /// contrasts against a live probe.
    pub last_error: Option<String>,
    pub last_error_at: Option<i64>,
    pub last_success_at: Option<i64>,
}

impl FromRow for Source {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            link: row.get(1)?,
            title: row.get(2)?,
            error_count: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
            etag: row.get(6)?,
            last_modified: row.get(7)?,
            next_fetch_at: row.get(8)?,
            last_error: row.get(9)?,
            last_error_at: row.get(10)?,
            last_success_at: row.get(11)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribe {
    pub id: i64,
    pub user_id: Option<i64>,
    pub source_id: Option<i64>,
    pub enable_notification: Option<i64>,
    pub enable_telegraph: Option<i64>,
    pub tag: Option<String>,
    pub interval: Option<i64>,
    pub wait_time: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl FromRow for Subscribe {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            user_id: row.get(1)?,
            source_id: row.get(2)?,
            enable_notification: row.get(3)?,
            enable_telegraph: row.get(4)?,
            tag: row.get(5)?,
            interval: row.get(6)?,
            wait_time: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
    pub source_id: Option<i64>,
    pub hash_id: String,
    pub raw_id: Option<String>,
    pub raw_link: Option<String>,
    pub title: Option<String>,
    pub telegraph_url: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl FromRow for Content {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            source_id: row.get(0)?,
            hash_id: row.get(1)?,
            raw_id: row.get(2)?,
            raw_link: row.get(3)?,
            title: row.get(4)?,
            telegraph_url: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}

/// A per-chat bookmark. Self-contained: `title`/`url`/`source_title` are
/// snapshots so a bookmark renders even after `contents`/`sources` are pruned;
/// `content_hash_id` is a breadcrumb only and may dangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub id: i64,
    pub chat_id: i64,
    pub created_by: i64,
    pub url: String,
    pub title: String,
    pub note: String,
    pub source_title: String,
    pub content_hash_id: Option<String>,
    pub telegraph_url: Option<String>,
    pub tag_state: i64,
    pub tag_attempts: i64,
    pub tag_next_attempt_at: i64,
    pub notify_message_id: Option<i64>,
    pub notify_kind: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl FromRow for Bookmark {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            chat_id: row.get(1)?,
            created_by: row.get(2)?,
            url: row.get(3)?,
            title: row.get(4)?,
            note: row.get(5)?,
            source_title: row.get(6)?,
            content_hash_id: row.get(7)?,
            telegraph_url: row.get(8)?,
            tag_state: row.get(9)?,
            tag_attempts: row.get(10)?,
            tag_next_attempt_at: row.get(11)?,
            notify_message_id: row.get(12)?,
            notify_kind: row.get(13)?,
            created_at: row.get(14)?,
            updated_at: row.get(15)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkTag {
    pub bookmark_id: i64,
    pub tag: String,
    pub origin: i64,
}

impl FromRow for BookmarkTag {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            bookmark_id: row.get(0)?,
            tag: row.get(1)?,
            origin: row.get(2)?,
        })
    }
}

/// One watched stock for a chat (migration 0006). `display_name`/`currency`/
/// `exchange` are snapshots taken at add time so the list still renders when
/// Yahoo is unavailable. No floats, so `Eq` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchItem {
    pub id: i64,
    pub chat_id: i64,
    pub created_by: i64,
    pub symbol: String,
    pub market: String,
    pub exchange: String,
    pub display_name: String,
    pub currency: String,
    pub note: String,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl FromRow for WatchItem {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            chat_id: row.get(1)?,
            created_by: row.get(2)?,
            symbol: row.get(3)?,
            market: row.get(4)?,
            exchange: row.get(5)?,
            display_name: row.get(6)?,
            currency: row.get(7)?,
            note: row.get(8)?,
            sort_order: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }
}

/// Per-symbol meta / session-clock cache row. Carries `f64` prices, so no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct StockMeta {
    pub symbol: String,
    pub market: String,
    pub exchange: String,
    pub display_name: String,
    pub currency: String,
    pub gmtoffset: i64,
    pub session_start: Option<i64>,
    pub session_end: Option<i64>,
    pub market_time: Option<i64>,
    pub last_price: Option<f64>,
    pub prev_close: Option<f64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    pub trade_date: String,
    pub source: String,
    pub fetched_at: i64,
    pub updated_at: i64,
}

impl FromRow for StockMeta {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            symbol: row.get(0)?,
            market: row.get(1)?,
            exchange: row.get(2)?,
            display_name: row.get(3)?,
            currency: row.get(4)?,
            gmtoffset: row.get(5)?,
            session_start: row.get(6)?,
            session_end: row.get(7)?,
            market_time: row.get(8)?,
            last_price: row.get(9)?,
            prev_close: row.get(10)?,
            week52_high: row.get(11)?,
            week52_low: row.get(12)?,
            trade_date: row.get(13)?,
            source: row.get(14)?,
            fetched_at: row.get(15)?,
            updated_at: row.get(16)?,
        })
    }
}

/// One OHLCV bar as stored (migration 0006). Carries `f64` prices, so no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct StockBar {
    pub symbol: String,
    pub trade_date: String,
    pub ts: i64,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<i64>,
    pub source: String,
    pub updated_at: i64,
}

impl FromRow for StockBar {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            symbol: row.get(0)?,
            trade_date: row.get(1)?,
            ts: row.get(2)?,
            open: row.get(3)?,
            high: row.get(4)?,
            low: row.get(5)?,
            close: row.get(6)?,
            volume: row.get(7)?,
            source: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }
}

/// Per-chat, per-market close-push setting. `push_minute` is minutes from
/// market-local midnight; `None` means "default delay after the close".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSetting {
    pub chat_id: i64,
    pub market: String,
    pub enabled: i64,
    pub push_minute: Option<i64>,
    pub updated_at: i64,
}

impl FromRow for PushSetting {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            chat_id: row.get(0)?,
            market: row.get(1)?,
            enabled: row.get(2)?,
            push_minute: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionRow {
    pub id: i64,
    pub name: Option<String>,
    pub value: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl FromRow for OptionRow {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            value: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }
}
