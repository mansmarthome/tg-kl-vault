use sqlx::FromRow;

/// GORM stored timestamps as SQLite datetime strings. Keep them as strings in
/// phase 1 until we verify a real production data.db sample.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct User {
    pub id: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
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
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
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

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
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

/// A per-chat bookmark. Self-contained: `title`/`url`/`source_title` are
/// snapshots so a bookmark renders even after `contents`/`sources` are pruned;
/// `content_hash_id` is a breadcrumb only and may dangle.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
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

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct BookmarkTag {
    pub bookmark_id: i64,
    pub tag: String,
    pub origin: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct OptionRow {
    pub id: i64,
    pub name: Option<String>,
    pub value: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}
