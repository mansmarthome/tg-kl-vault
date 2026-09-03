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
    pub enable_source_title: Option<i64>,
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

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct OptionRow {
    pub id: i64,
    pub name: Option<String>,
    pub value: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}
