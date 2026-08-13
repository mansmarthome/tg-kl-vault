use std::collections::HashSet;
use std::sync::Arc;

use libsql::{params::IntoParams, Connection, Row, Value};

use super::models::{Content, Source, Subscribe, User};
use super::{Db, DbResult, FromRow};
use crate::config::ERROR_THRESHOLD;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionSource {
    pub subscribe_id: i64,
    pub user_id: Option<i64>,
    pub source_id: Option<i64>,
    pub enable_notification: Option<i64>,
    pub enable_telegraph: Option<i64>,
    pub tag: Option<String>,
    pub interval: Option<i64>,
    pub wait_time: Option<i64>,
    pub link: Option<String>,
    pub title: Option<String>,
    // Source health, joined in so `/list` and `/feedcheck` need one query
    // instead of an N+1 `get_source` loop.
    pub error_count: Option<i64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<i64>,
    pub last_success_at: Option<i64>,
}

impl SubscriptionSource {
    /// Mirrors `sources_due`'s pause/give-up gate: the scheduler skips these
    /// entirely, which is exactly the state a user needs surfaced.
    pub fn is_paused(&self) -> bool {
        self.error_count.unwrap_or(0) >= i64::from(ERROR_THRESHOLD)
    }
}

impl FromRow for SubscriptionSource {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok(Self {
            subscribe_id: row.get(0)?,
            user_id: row.get(1)?,
            source_id: row.get(2)?,
            enable_notification: row.get(3)?,
            enable_telegraph: row.get(4)?,
            tag: row.get(5)?,
            interval: row.get(6)?,
            wait_time: row.get(7)?,
            link: row.get(8)?,
            title: row.get(9)?,
            error_count: row.get(10)?,
            etag: row.get(11)?,
            last_modified: row.get(12)?,
            last_error: row.get(13)?,
            last_error_at: row.get(14)?,
            last_success_at: row.get(15)?,
        })
    }
}

#[derive(Clone)]
pub struct Repo {
    db: Arc<Db>,
}

impl Repo {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    pub fn conn(&self) -> &Connection {
        self.db.conn()
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    // --- small query helpers replacing sqlx's typed query API ---

    pub(crate) async fn exec(&self, sql: &str, params: impl IntoParams) -> DbResult<u64> {
        self.conn().execute(sql, params).await
    }

    pub(crate) async fn query_all<T: FromRow>(
        &self,
        sql: &str,
        params: impl IntoParams,
    ) -> DbResult<Vec<T>> {
        let mut rows = self.conn().query(sql, params).await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(T::from_row(&row)?);
        }
        Ok(out)
    }

    pub(crate) async fn query_opt<T: FromRow>(
        &self,
        sql: &str,
        params: impl IntoParams,
    ) -> DbResult<Option<T>> {
        let mut rows = self.conn().query(sql, params).await?;
        match rows.next().await? {
            Some(row) => Ok(Some(T::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn scalar_i64(&self, sql: &str, params: impl IntoParams) -> DbResult<i64> {
        let mut rows = self.conn().query(sql, params).await?;
        let row = rows.next().await?.ok_or(libsql::Error::QueryReturnedNoRows)?;
        row.get::<i64>(0)
    }

    pub(crate) async fn scalar_opt_i64(
        &self,
        sql: &str,
        params: impl IntoParams,
    ) -> DbResult<Option<i64>> {
        let mut rows = self.conn().query(sql, params).await?;
        match rows.next().await? {
            Some(row) => row.get::<Option<i64>>(0),
            None => Ok(None),
        }
    }

    pub(crate) async fn scalar_opt_string(
        &self,
        sql: &str,
        params: impl IntoParams,
    ) -> DbResult<Option<String>> {
        let mut rows = self.conn().query(sql, params).await?;
        match rows.next().await? {
            Some(row) => row.get::<Option<String>>(0),
            None => Ok(None),
        }
    }

    pub(crate) async fn scalar_all_string(
        &self,
        sql: &str,
        params: impl IntoParams,
    ) -> DbResult<Vec<String>> {
        let mut rows = self.conn().query(sql, params).await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row.get::<String>(0)?);
        }
        Ok(out)
    }

    pub(crate) async fn scalar_all_i64(
        &self,
        sql: &str,
        params: impl IntoParams,
    ) -> DbResult<Vec<i64>> {
        let mut rows = self.conn().query(sql, params).await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row.get::<i64>(0)?);
        }
        Ok(out)
    }

    // --- repository methods ---

    pub async fn get_user(&self, id: i64) -> DbResult<Option<User>> {
        self.query_opt::<User>(
            "SELECT id, created_at, updated_at FROM users WHERE id = ?",
            libsql::params![id],
        )
        .await
    }

    pub async fn ensure_user(&self, id: i64) -> DbResult<()> {
        self.exec(
            "INSERT OR IGNORE INTO users (id, created_at, updated_at) VALUES (?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            libsql::params![id],
        )
        .await?;
        Ok(())
    }

    pub async fn list_sources(&self) -> DbResult<Vec<Source>> {
        self.query_all::<Source>(
            "SELECT id, link, title, error_count, created_at, updated_at, etag, last_modified, next_fetch_at, \
                    last_error, last_error_at, last_success_at \
             FROM sources ORDER BY id",
            (),
        )
        .await
    }

    pub async fn get_source(&self, id: i64) -> DbResult<Option<Source>> {
        self.query_opt::<Source>(
            "SELECT id, link, title, error_count, created_at, updated_at, etag, last_modified, next_fetch_at, \
                    last_error, last_error_at, last_success_at \
             FROM sources WHERE id = ?",
            libsql::params![id],
        )
        .await
    }

    pub async fn source_by_link(&self, link: &str) -> DbResult<Option<Source>> {
        self.query_opt::<Source>(
            "SELECT id, link, title, error_count, created_at, updated_at, etag, last_modified, next_fetch_at, \
                    last_error, last_error_at, last_success_at \
             FROM sources WHERE link = ? LIMIT 1",
            libsql::params![link],
        )
        .await
    }

    pub async fn insert_source(&self, link: &str, title: &str) -> DbResult<i64> {
        self.exec(
            "INSERT INTO sources (link, title, error_count, created_at, updated_at, next_fetch_at) \
             VALUES (?, ?, 0, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, 0)",
            libsql::params![link, title],
        )
        .await?;
        Ok(self.conn().last_insert_rowid())
    }

    pub async fn sources_due(&self, now: i64, limit: i64) -> DbResult<Vec<Source>> {
        self.query_all::<Source>(
            "SELECT id, link, title, error_count, created_at, updated_at, etag, last_modified, next_fetch_at, \
                    last_error, last_error_at, last_success_at \
             FROM sources \
             WHERE COALESCE(next_fetch_at, 0) <= ? AND COALESCE(error_count, 0) < 100 \
             ORDER BY COALESCE(next_fetch_at, 0), id LIMIT ?",
            libsql::params![now, limit],
        )
        .await
    }

    pub async fn subscribe_user(&self, user_id: i64, source_id: i64) -> DbResult<bool> {
        self.ensure_user(user_id).await?;
        if self.subscription(user_id, source_id).await?.is_some() {
            return Ok(false);
        }
        self.exec(
            "INSERT INTO subscribes \
             (user_id, source_id, enable_notification, enable_telegraph, tag, interval, wait_time, created_at, updated_at) \
             VALUES (?, ?, 1, 1, '', 0, 0, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            libsql::params![user_id, source_id],
        )
        .await?;
        Ok(true)
    }

    pub async fn subscription(
        &self,
        user_id: i64,
        source_id: i64,
    ) -> DbResult<Option<Subscribe>> {
        self.query_opt::<Subscribe>(
            "SELECT id, user_id, source_id, enable_notification, enable_telegraph, tag, interval, wait_time, created_at, updated_at \
             FROM subscribes WHERE user_id = ? AND source_id = ? LIMIT 1",
            libsql::params![user_id, source_id],
        )
        .await
    }

    /// Port of Go's `Core.Unsubscribe`: removes the subscription, then removes
    /// the source (and its dedup content ledger) once it has no subscribers
    /// left, matching `removeSource`.
    pub async fn unsubscribe_user(&self, user_id: i64, source_id: i64) -> DbResult<bool> {
        let affected = self
            .exec(
                "DELETE FROM subscribes WHERE user_id = ? AND source_id = ?",
                libsql::params![user_id, source_id],
            )
            .await?;
        if affected == 0 {
            return Ok(false);
        }
        if self.count_source_subscriptions(source_id).await? == 0 {
            self.delete_source_and_contents(source_id).await?;
        }
        Ok(true)
    }

    /// Port of Go's `Core.UnsubscribeAllSource`: same per-source cascade as
    /// `unsubscribe_user`, applied to every source the user is subscribed to.
    pub async fn unsubscribe_all_user(&self, user_id: i64) -> DbResult<u64> {
        let source_ids = self
            .scalar_all_i64(
                "SELECT DISTINCT source_id FROM subscribes WHERE user_id = ? AND source_id IS NOT NULL",
                libsql::params![user_id],
            )
            .await?;

        let affected = self
            .exec(
                "DELETE FROM subscribes WHERE user_id = ?",
                libsql::params![user_id],
            )
            .await?;

        for source_id in source_ids {
            if self.count_source_subscriptions(source_id).await? == 0 {
                self.delete_source_and_contents(source_id).await?;
            }
        }
        Ok(affected)
    }

    pub async fn count_source_subscriptions(&self, source_id: i64) -> DbResult<i64> {
        self.scalar_i64(
            "SELECT COUNT(*) FROM subscribes WHERE source_id = ?",
            libsql::params![source_id],
        )
        .await
    }

    pub async fn delete_source_and_contents(&self, source_id: i64) -> DbResult<()> {
        self.exec(
            "DELETE FROM contents WHERE source_id = ?",
            libsql::params![source_id],
        )
        .await?;
        self.exec(
            "DELETE FROM sources WHERE id = ?",
            libsql::params![source_id],
        )
        .await?;
        Ok(())
    }

    pub async fn subscriptions_for_user(
        &self,
        user_id: i64,
    ) -> DbResult<Vec<SubscriptionSource>> {
        self.query_all::<SubscriptionSource>(
            "SELECT subscribes.id AS subscribe_id, subscribes.user_id, subscribes.source_id, \
                    subscribes.enable_notification, subscribes.enable_telegraph, subscribes.tag, \
                    subscribes.interval, subscribes.wait_time, sources.link, sources.title, \
                    sources.error_count, sources.etag, sources.last_modified, \
                    sources.last_error, sources.last_error_at, sources.last_success_at \
             FROM subscribes JOIN sources ON sources.id = subscribes.source_id \
             WHERE subscribes.user_id = ? ORDER BY sources.id",
            libsql::params![user_id],
        )
        .await
    }

    pub async fn mark_user_sources_due(&self, user_id: i64) -> DbResult<u64> {
        self.exec(
            "UPDATE sources \
             SET next_fetch_at = 0, error_count = 0, updated_at = CURRENT_TIMESTAMP \
             WHERE id IN (SELECT source_id FROM subscribes WHERE user_id = ? AND source_id IS NOT NULL)",
            libsql::params![user_id],
        )
        .await
    }

    pub async fn get_option(&self, name: &str) -> DbResult<Option<String>> {
        self.scalar_opt_string(
            "SELECT value FROM options WHERE name = ? ORDER BY id DESC LIMIT 1",
            libsql::params![name],
        )
        .await
    }

    /// Chat ids whose option `{prefix}{chat_id}` is explicitly off (value
    /// `"0"`). Opt-out semantics: absence of a row means the feature is on, so
    /// this returns only the (usually few) chats that turned it off.
    pub async fn chat_ids_with_option_off(&self, prefix: &str) -> DbResult<HashSet<i64>> {
        // Escape LIKE metacharacters in the (trusted, but let's be tidy) prefix.
        let mut pattern = String::with_capacity(prefix.len() + 1);
        for ch in prefix.chars() {
            if matches!(ch, '\\' | '%' | '_') {
                pattern.push('\\');
            }
            pattern.push(ch);
        }
        pattern.push('%');
        let names = self
            .scalar_all_string(
                "SELECT name FROM options WHERE name LIKE ? ESCAPE '\\' AND value = '0'",
                libsql::params![pattern],
            )
            .await?;
        Ok(names
            .into_iter()
            .filter_map(|name| name.strip_prefix(prefix).and_then(|s| s.parse::<i64>().ok()))
            .collect())
    }

    pub async fn set_option(&self, name: &str, value: &str) -> DbResult<()> {
        self.exec(
            "INSERT INTO options (name, value, created_at, updated_at) VALUES (?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP) \
             ON CONFLICT(name) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
            libsql::params![name, value],
        )
        .await?;
        Ok(())
    }

    pub async fn set_subscription_tag(
        &self,
        user_id: i64,
        source_id: i64,
        tag: &str,
    ) -> DbResult<bool> {
        let affected = self
            .exec(
                "UPDATE subscribes SET tag = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND source_id = ?",
                libsql::params![tag, user_id, source_id],
            )
            .await?;
        Ok(affected > 0)
    }

    pub async fn set_subscription_interval(
        &self,
        user_id: i64,
        source_id: i64,
        interval: i64,
    ) -> DbResult<bool> {
        let affected = self
            .exec(
                "UPDATE subscribes SET interval = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND source_id = ?",
                libsql::params![interval, user_id, source_id],
            )
            .await?;
        Ok(affected > 0)
    }

    pub async fn set_all_subscription_interval(
        &self,
        user_id: i64,
        interval: i64,
    ) -> DbResult<u64> {
        self.exec(
            "UPDATE subscribes SET interval = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ?",
            libsql::params![interval, user_id],
        )
        .await
    }

    pub async fn toggle_subscription_notice(
        &self,
        user_id: i64,
        source_id: i64,
    ) -> DbResult<Option<Subscribe>> {
        let Some(sub) = self.subscription(user_id, source_id).await? else {
            return Ok(None);
        };
        let new_value = if sub.enable_notification == Some(1) {
            0
        } else {
            1
        };
        self.exec(
            "UPDATE subscribes SET enable_notification = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE user_id = ? AND source_id = ?",
            libsql::params![new_value, user_id, source_id],
        )
        .await?;
        self.subscription(user_id, source_id).await
    }

    pub async fn toggle_subscription_telegraph(
        &self,
        user_id: i64,
        source_id: i64,
    ) -> DbResult<Option<Subscribe>> {
        let Some(sub) = self.subscription(user_id, source_id).await? else {
            return Ok(None);
        };
        let new_value = if sub.enable_telegraph == Some(1) {
            0
        } else {
            1
        };
        self.exec(
            "UPDATE subscribes SET enable_telegraph = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE user_id = ? AND source_id = ?",
            libsql::params![new_value, user_id, source_id],
        )
        .await?;
        self.subscription(user_id, source_id).await
    }

    /// Port of Go's `Core.EnableSourceUpdate` / `ClearSourceErrorCount`: this
    /// pauses/resumes the *source* for all its subscribers (not a per-user
    /// flag), by clearing its `error_count` below `ERROR_THRESHOLD`.
    pub async fn enable_source_update(&self, source_id: i64) -> DbResult<()> {
        self.exec(
            "UPDATE sources SET error_count = 0, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            libsql::params![source_id],
        )
        .await?;
        Ok(())
    }

    /// Port of Go's `Core.DisableSourceUpdate`: sets `error_count` one past
    /// `ERROR_THRESHOLD` so the scheduler's `sources_due` query skips it.
    pub async fn disable_source_update(&self, source_id: i64) -> DbResult<()> {
        self.exec(
            "UPDATE sources SET error_count = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            libsql::params![i64::from(ERROR_THRESHOLD) + 1, source_id],
        )
        .await?;
        Ok(())
    }

    /// Port of Go's `Core.ToggleSourceUpdateStatus`.
    pub async fn toggle_source_update_status(
        &self,
        source_id: i64,
    ) -> DbResult<Option<Source>> {
        let Some(source) = self.get_source(source_id).await? else {
            return Ok(None);
        };
        if source.error_count.unwrap_or(0) < i64::from(ERROR_THRESHOLD) {
            self.disable_source_update(source_id).await?;
        } else {
            self.enable_source_update(source_id).await?;
        }
        self.get_source(source_id).await
    }

    pub async fn subscribes_for_source(&self, source_id: i64) -> DbResult<Vec<Subscribe>> {
        self.query_all::<Subscribe>(
            "SELECT id, user_id, source_id, enable_notification, enable_telegraph, tag, interval, wait_time, created_at, updated_at \
             FROM subscribes WHERE source_id = ? ORDER BY id",
            libsql::params![source_id],
        )
        .await
    }

    pub async fn existing_hash_ids(
        &self,
        source_id: i64,
        hash_ids: &[String],
    ) -> DbResult<HashSet<String>> {
        let mut found = HashSet::new();
        for chunk in hash_ids.chunks(500) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT hash_id FROM contents WHERE source_id = ? AND hash_id IN ({placeholders})"
            );
            let mut params: Vec<Value> = Vec::with_capacity(chunk.len() + 1);
            params.push(Value::from(source_id));
            for hash in chunk {
                params.push(Value::from(hash.clone()));
            }
            found.extend(self.scalar_all_string(&sql, params).await?);
        }
        Ok(found)
    }

    pub async fn insert_content(&self, content: &Content) -> DbResult<()> {
        self.exec(
            "INSERT OR IGNORE INTO contents \
             (source_id, hash_id, raw_id, raw_link, title, telegraph_url, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            libsql::params![
                content.source_id,
                content.hash_id.as_str(),
                content.raw_id.as_deref(),
                content.raw_link.as_deref(),
                content.title.as_deref(),
                content.telegraph_url.as_deref(),
            ],
        )
        .await?;
        Ok(())
    }

    /// Records a failed fetch/parse. `error` is kept (truncated) so `/feedcheck`
    /// can tell the user *why* a feed stopped working — before migration 0005
    /// the message only ever reached the server log.
    pub async fn mark_source_error(
        &self,
        source_id: i64,
        next_fetch_at: i64,
        error: &str,
    ) -> DbResult<()> {
        self.exec(
            "UPDATE sources \
             SET error_count = COALESCE(error_count, 0) + 1, next_fetch_at = ?, \
                 last_error = ?, last_error_at = CAST(strftime('%s', 'now') AS INTEGER), \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
            libsql::params![next_fetch_at, truncate_error(error), source_id],
        )
        .await?;
        Ok(())
    }

    pub async fn mark_source_success(
        &self,
        source_id: i64,
        etag: Option<&str>,
        last_modified: Option<&str>,
        next_fetch_at: i64,
    ) -> DbResult<()> {
        self.exec(
            "UPDATE sources \
             SET error_count = 0, etag = COALESCE(?, etag), last_modified = COALESCE(?, last_modified), \
                 next_fetch_at = ?, last_error = NULL, last_error_at = NULL, \
                 last_success_at = CAST(strftime('%s', 'now') AS INTEGER), \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
            libsql::params![etag, last_modified, next_fetch_at, source_id],
        )
        .await?;
        Ok(())
    }

    pub async fn prune_contents(
        &self,
        source_id: i64,
        retention_days: u32,
        keep_recent: u32,
    ) -> DbResult<u64> {
        let modifier = format!("-{} days", retention_days);
        self.exec(
            "DELETE FROM contents \
             WHERE source_id = ? \
               AND created_at < datetime('now', ?) \
               AND hash_id NOT IN ( \
                 SELECT hash_id FROM contents WHERE source_id = ? ORDER BY created_at DESC LIMIT ? \
               )",
            libsql::params![source_id, modifier, source_id, i64::from(keep_recent)],
        )
        .await
    }
}

/// Feed errors can carry a whole response body; cap what lands in `last_error`
/// so one bad source cannot bloat every `SELECT` on `sources`.
fn truncate_error(error: &str) -> String {
    const MAX: usize = 300;
    let error = error.trim();
    match error.char_indices().nth(MAX) {
        Some((idx, _)) => format!("{}…", &error[..idx]),
        None => error.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn repo_opens_fresh_db_and_dedups_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        repo.exec(
            "INSERT INTO sources (id, link, title, error_count, next_fetch_at) VALUES (1, 'https://e.test/feed', 'E', 0, 0)",
            (),
        )
        .await
        .unwrap();

        let due = repo.sources_due(0, 10).await.unwrap();
        assert_eq!(due.len(), 1);

        repo.insert_content(&Content {
            source_id: Some(1),
            hash_id: "abc123".to_owned(),
            raw_id: Some("guid".to_owned()),
            raw_link: Some("https://e.test/1".to_owned()),
            title: Some("hello".to_owned()),
            telegraph_url: None,
            created_at: None,
            updated_at: None,
        })
        .await
        .unwrap();

        let found = repo
            .existing_hash_ids(1, &["abc123".to_owned(), "missing".to_owned()])
            .await
            .unwrap();
        assert!(found.contains("abc123"));
        assert!(!found.contains("missing"));
    }

    #[tokio::test]
    async fn repo_ensures_users_and_inserts_sources() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        repo.ensure_user(-100).await.unwrap();
        repo.ensure_user(-100).await.unwrap();
        assert_eq!(repo.get_user(-100).await.unwrap().unwrap().id, -100);

        let source_id = repo
            .insert_source("https://example.com/feed", "Example")
            .await
            .unwrap();
        assert_eq!(source_id, 1);
        assert_eq!(
            repo.source_by_link("https://example.com/feed")
                .await
                .unwrap()
                .unwrap()
                .id,
            1
        );
        assert_eq!(repo.list_sources().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn subscription_crud_methods_work() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        let source_id = repo
            .insert_source("https://example.com/feed", "Example")
            .await
            .unwrap();
        assert!(repo.subscribe_user(42, source_id).await.unwrap());
        assert!(!repo.subscribe_user(42, source_id).await.unwrap());
        assert_eq!(repo.subscriptions_for_user(42).await.unwrap().len(), 1);
        assert!(repo
            .set_subscription_tag(42, source_id, "#tag")
            .await
            .unwrap());
        assert!(repo
            .set_subscription_interval(42, source_id, 30)
            .await
            .unwrap());
        assert!(repo.unsubscribe_user(42, source_id).await.unwrap());
        assert!(!repo.unsubscribe_user(42, source_id).await.unwrap());
    }

    #[tokio::test]
    async fn mark_user_sources_due_resumes_and_schedules_now() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        let source_id = repo
            .insert_source("https://example.com/feed", "Example")
            .await
            .unwrap();
        repo.subscribe_user(42, source_id).await.unwrap();
        repo.exec(
            "UPDATE sources SET next_fetch_at = 999999, error_count = 101 WHERE id = ?",
            libsql::params![source_id],
        )
        .await
        .unwrap();

        assert_eq!(repo.mark_user_sources_due(42).await.unwrap(), 1);
        let source = repo.get_source(source_id).await.unwrap().unwrap();
        assert_eq!(source.next_fetch_at, 0);
        assert_eq!(source.error_count, Some(0));
    }

    /// The error text used to exist only in the server log, so a user could
    /// never find out *why* a feed went quiet. A later success must clear it,
    /// otherwise `/feedcheck` would keep reporting a resolved failure.
    #[tokio::test]
    async fn source_error_is_recorded_and_cleared_by_the_next_success() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        let source_id = repo.insert_source("https://example.com/feed", "Example").await.unwrap();

        repo.mark_source_error(source_id, 123, "HTTP status client error (404 Not Found)")
            .await
            .unwrap();
        let source = repo.get_source(source_id).await.unwrap().unwrap();
        assert_eq!(source.error_count, Some(1));
        assert_eq!(
            source.last_error.as_deref(),
            Some("HTTP status client error (404 Not Found)")
        );
        assert!(source.last_error_at.unwrap_or(0) > 0);
        assert_eq!(source.last_success_at, None);

        repo.mark_source_success(source_id, Some("etag-1"), None, 456).await.unwrap();
        let source = repo.get_source(source_id).await.unwrap().unwrap();
        assert_eq!(source.error_count, Some(0));
        assert_eq!(source.last_error, None);
        assert_eq!(source.last_error_at, None);
        assert!(source.last_success_at.unwrap_or(0) > 0);
    }

    #[test]
    fn long_errors_are_truncated_on_a_char_boundary() {
        // A multi-byte error longer than the cap must not panic on slicing.
        let long = "錯".repeat(500);
        let truncated = truncate_error(&long);
        assert_eq!(truncated.chars().count(), 301, "300 chars plus the ellipsis");
        assert!(truncated.ends_with('…'));
        assert_eq!(truncate_error("  short  "), "short");
    }

    /// `/list` and `/feedcheck` read health off the subscription join rather
    /// than issuing an N+1 `get_source` per row.
    #[tokio::test]
    async fn subscriptions_for_user_carries_source_health() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        let source_id = repo.insert_source("https://example.com/feed", "Example").await.unwrap();
        repo.subscribe_user(42, source_id).await.unwrap();
        repo.mark_source_error(source_id, 123, "boom").await.unwrap();

        let subs = repo.subscriptions_for_user(42).await.unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].error_count, Some(1));
        assert_eq!(subs[0].last_error.as_deref(), Some("boom"));
        assert!(!subs[0].is_paused());

        repo.disable_source_update(source_id).await.unwrap();
        let subs = repo.subscriptions_for_user(42).await.unwrap();
        assert!(subs[0].is_paused(), "a paused source is invisible to the scheduler");
    }

    #[tokio::test]
    async fn unsubscribe_cascades_to_orphan_source() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        let source_id = repo
            .insert_source("https://example.com/feed", "Example")
            .await
            .unwrap();
        repo.subscribe_user(1, source_id).await.unwrap();
        repo.subscribe_user(2, source_id).await.unwrap();
        repo.insert_content(&Content {
            source_id: Some(source_id),
            hash_id: "h1".to_owned(),
            raw_id: None,
            raw_link: None,
            title: None,
            telegraph_url: None,
            created_at: None,
            updated_at: None,
        })
        .await
        .unwrap();

        // Still has a subscriber left: source and its contents survive.
        assert!(repo.unsubscribe_user(1, source_id).await.unwrap());
        assert!(repo.get_source(source_id).await.unwrap().is_some());

        // Last subscriber leaves: source and its dedup ledger are removed.
        assert!(repo.unsubscribe_user(2, source_id).await.unwrap());
        assert!(repo.get_source(source_id).await.unwrap().is_none());
        assert!(repo
            .existing_hash_ids(source_id, &["h1".to_owned()])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn source_update_toggle_matches_error_threshold_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        let source_id = repo
            .insert_source("https://example.com/feed", "Example")
            .await
            .unwrap();
        assert_eq!(
            repo.get_source(source_id)
                .await
                .unwrap()
                .unwrap()
                .error_count,
            Some(0)
        );

        let paused = repo
            .toggle_source_update_status(source_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(paused.error_count, Some(101));

        let resumed = repo
            .toggle_source_update_status(source_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resumed.error_count, Some(0));

        repo.disable_source_update(source_id).await.unwrap();
        assert_eq!(
            repo.get_source(source_id)
                .await
                .unwrap()
                .unwrap()
                .error_count,
            Some(101)
        );
        repo.enable_source_update(source_id).await.unwrap();
        assert_eq!(
            repo.get_source(source_id)
                .await
                .unwrap()
                .unwrap()
                .error_count,
            Some(0)
        );
    }

    #[tokio::test]
    async fn subscription_notice_and_telegraph_toggle() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        let source_id = repo
            .insert_source("https://example.com/feed", "Example")
            .await
            .unwrap();
        repo.subscribe_user(42, source_id).await.unwrap();

        let sub = repo
            .toggle_subscription_notice(42, source_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sub.enable_notification, Some(0));
        let sub = repo
            .toggle_subscription_notice(42, source_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sub.enable_notification, Some(1));

        let sub = repo
            .toggle_subscription_telegraph(42, source_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sub.enable_telegraph, Some(0));
        let sub = repo
            .toggle_subscription_telegraph(42, source_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sub.enable_telegraph, Some(1));
    }

    #[tokio::test]
    async fn prune_contents_keeps_recent_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(db);

        repo.exec(
            "INSERT INTO sources (id, link, title, error_count, next_fetch_at) VALUES (1, 'https://e.test/feed', 'E', 0, 0)",
            (),
        )
        .await
        .unwrap();

        for i in 0..5 {
            repo.exec(
                "INSERT INTO contents (source_id, hash_id, title, created_at, updated_at) VALUES (1, ?, ?, ?, ?)",
                libsql::params![
                    format!("h{i}"),
                    format!("title {i}"),
                    format!("2020-01-0{} 00:00:00", i + 1),
                    format!("2020-01-0{} 00:00:00", i + 1),
                ],
            )
            .await
            .unwrap();
        }

        let deleted = repo.prune_contents(1, 1, 2).await.unwrap();
        assert_eq!(deleted, 3);
        let remaining = repo
            .existing_hash_ids(
                1,
                &[
                    "h0".into(),
                    "h1".into(),
                    "h2".into(),
                    "h3".into(),
                    "h4".into(),
                ],
            )
            .await
            .unwrap();
        assert_eq!(remaining, HashSet::from(["h3".to_owned(), "h4".to_owned()]));
    }
}
