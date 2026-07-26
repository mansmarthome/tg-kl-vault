use std::collections::HashSet;

use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use super::models::{Content, Source, Subscribe, User};

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
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
}

#[derive(Debug, Clone)]
pub struct Repo {
    pool: SqlitePool,
}

impl Repo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn get_user(&self, id: i64) -> sqlx::Result<Option<User>> {
        sqlx::query_as::<_, User>("SELECT id, created_at, updated_at FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn ensure_user(&self, id: i64) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO users (id, created_at, updated_at) VALUES (?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_sources(&self) -> sqlx::Result<Vec<Source>> {
        sqlx::query_as::<_, Source>(
            "SELECT id, link, title, error_count, created_at, updated_at, etag, last_modified, next_fetch_at \
             FROM sources ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn source_by_link(&self, link: &str) -> sqlx::Result<Option<Source>> {
        sqlx::query_as::<_, Source>(
            "SELECT id, link, title, error_count, created_at, updated_at, etag, last_modified, next_fetch_at \
             FROM sources WHERE link = ? LIMIT 1",
        )
        .bind(link)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn insert_source(&self, link: &str, title: &str) -> sqlx::Result<i64> {
        let result = sqlx::query(
            "INSERT INTO sources (link, title, error_count, created_at, updated_at, next_fetch_at) \
             VALUES (?, ?, 0, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, 0)",
        )
        .bind(link)
        .bind(title)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn sources_due(&self, now: i64, limit: i64) -> sqlx::Result<Vec<Source>> {
        sqlx::query_as::<_, Source>(
            "SELECT id, link, title, error_count, created_at, updated_at, etag, last_modified, next_fetch_at \
             FROM sources \
             WHERE COALESCE(next_fetch_at, 0) <= ? AND COALESCE(error_count, 0) < 100 \
             ORDER BY COALESCE(next_fetch_at, 0), id LIMIT ?",
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn subscribe_user(&self, user_id: i64, source_id: i64) -> sqlx::Result<bool> {
        self.ensure_user(user_id).await?;
        if self.subscription(user_id, source_id).await?.is_some() {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO subscribes \
             (user_id, source_id, enable_notification, enable_telegraph, tag, interval, wait_time, created_at, updated_at) \
             VALUES (?, ?, 1, 1, '', 0, 0, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        )
        .bind(user_id)
        .bind(source_id)
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    pub async fn subscription(&self, user_id: i64, source_id: i64) -> sqlx::Result<Option<Subscribe>> {
        sqlx::query_as::<_, Subscribe>(
            "SELECT id, user_id, source_id, enable_notification, enable_telegraph, tag, interval, wait_time, created_at, updated_at \
             FROM subscribes WHERE user_id = ? AND source_id = ? LIMIT 1",
        )
        .bind(user_id)
        .bind(source_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn unsubscribe_user(&self, user_id: i64, source_id: i64) -> sqlx::Result<bool> {
        let result = sqlx::query("DELETE FROM subscribes WHERE user_id = ? AND source_id = ?")
            .bind(user_id)
            .bind(source_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn unsubscribe_all_user(&self, user_id: i64) -> sqlx::Result<u64> {
        let result = sqlx::query("DELETE FROM subscribes WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn subscriptions_for_user(&self, user_id: i64) -> sqlx::Result<Vec<SubscriptionSource>> {
        sqlx::query_as::<_, SubscriptionSource>(
            "SELECT subscribes.id AS subscribe_id, subscribes.user_id, subscribes.source_id, \
                    subscribes.enable_notification, subscribes.enable_telegraph, subscribes.tag, \
                    subscribes.interval, subscribes.wait_time, sources.link, sources.title \
             FROM subscribes JOIN sources ON sources.id = subscribes.source_id \
             WHERE subscribes.user_id = ? ORDER BY sources.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn set_subscription_tag(&self, user_id: i64, source_id: i64, tag: &str) -> sqlx::Result<bool> {
        let result = sqlx::query(
            "UPDATE subscribes SET tag = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND source_id = ?",
        )
        .bind(tag)
        .bind(user_id)
        .bind(source_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_subscription_interval(&self, user_id: i64, source_id: i64, interval: i64) -> sqlx::Result<bool> {
        let result = sqlx::query(
            "UPDATE subscribes SET interval = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND source_id = ?",
        )
        .bind(interval)
        .bind(user_id)
        .bind(source_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_all_enabled(&self, user_id: i64, enabled: bool) -> sqlx::Result<u64> {
        let result = sqlx::query(
            "UPDATE subscribes SET wait_time = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ?",
        )
        .bind(if enabled { 0 } else { 1 })
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn subscribes_for_source(&self, source_id: i64) -> sqlx::Result<Vec<Subscribe>> {
        sqlx::query_as::<_, Subscribe>(
            "SELECT id, user_id, source_id, enable_notification, enable_telegraph, tag, interval, wait_time, created_at, updated_at \
             FROM subscribes WHERE source_id = ? ORDER BY id",
        )
        .bind(source_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn existing_hash_ids(
        &self,
        source_id: i64,
        hash_ids: &[String],
    ) -> sqlx::Result<HashSet<String>> {
        let mut found = HashSet::new();
        for chunk in hash_ids.chunks(500) {
            if chunk.is_empty() {
                continue;
            }
            let mut builder =
                QueryBuilder::<Sqlite>::new("SELECT hash_id FROM contents WHERE source_id = ");
            builder.push_bind(source_id).push(" AND hash_id IN (");
            let mut separated = builder.separated(",");
            for hash in chunk {
                separated.push_bind(hash);
            }
            separated.push_unseparated(")");
            found.extend(builder.build_query_scalar::<String>().fetch_all(&self.pool).await?);
        }
        Ok(found)
    }

    pub async fn insert_content(&self, content: &Content) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO contents \
             (source_id, hash_id, raw_id, raw_link, title, telegraph_url, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        )
        .bind(content.source_id)
        .bind(&content.hash_id)
        .bind(&content.raw_id)
        .bind(&content.raw_link)
        .bind(&content.title)
        .bind(&content.telegraph_url)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_source_error(&self, source_id: i64, next_fetch_at: i64) -> sqlx::Result<()> {
        sqlx::query(
            "UPDATE sources \
             SET error_count = COALESCE(error_count, 0) + 1, next_fetch_at = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(next_fetch_at)
        .bind(source_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_source_success(
        &self,
        source_id: i64,
        etag: Option<&str>,
        last_modified: Option<&str>,
        next_fetch_at: i64,
    ) -> sqlx::Result<()> {
        sqlx::query(
            "UPDATE sources \
             SET error_count = 0, etag = COALESCE(?, etag), last_modified = COALESCE(?, last_modified), \
                 next_fetch_at = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(etag)
        .bind(last_modified)
        .bind(next_fetch_at)
        .bind(source_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn prune_contents(
        &self,
        source_id: i64,
        retention_days: u32,
        keep_recent: u32,
    ) -> sqlx::Result<u64> {
        let modifier = format!("-{} days", retention_days);
        let result = sqlx::query(
            "DELETE FROM contents \
             WHERE source_id = ? \
               AND created_at < datetime('now', ?) \
               AND hash_id NOT IN ( \
                 SELECT hash_id FROM contents WHERE source_id = ? ORDER BY created_at DESC LIMIT ? \
               )",
        )
        .bind(source_id)
        .bind(modifier)
        .bind(source_id)
        .bind(i64::from(keep_recent))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
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
        let pool = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool.clone());

        sqlx::query("INSERT INTO sources (id, link, title, error_count, next_fetch_at) VALUES (1, 'https://e.test/feed', 'E', 0, 0)")
            .execute(&pool)
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
        let pool = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool);

        repo.ensure_user(-100).await.unwrap();
        repo.ensure_user(-100).await.unwrap();
        assert_eq!(repo.get_user(-100).await.unwrap().unwrap().id, -100);

        let source_id = repo.insert_source("https://example.com/feed", "Example").await.unwrap();
        assert_eq!(source_id, 1);
        assert_eq!(repo.source_by_link("https://example.com/feed").await.unwrap().unwrap().id, 1);
        assert_eq!(repo.list_sources().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn subscription_crud_methods_work() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let pool = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool);

        let source_id = repo.insert_source("https://example.com/feed", "Example").await.unwrap();
        assert!(repo.subscribe_user(42, source_id).await.unwrap());
        assert!(!repo.subscribe_user(42, source_id).await.unwrap());
        assert_eq!(repo.subscriptions_for_user(42).await.unwrap().len(), 1);
        assert!(repo.set_subscription_tag(42, source_id, "#tag").await.unwrap());
        assert!(repo.set_subscription_interval(42, source_id, 30).await.unwrap());
        assert_eq!(repo.set_all_enabled(42, false).await.unwrap(), 1);
        assert!(repo.unsubscribe_user(42, source_id).await.unwrap());
        assert!(!repo.unsubscribe_user(42, source_id).await.unwrap());
    }

    #[tokio::test]
    async fn prune_contents_keeps_recent_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let pool = db::connect(db_path.to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool.clone());

        sqlx::query("INSERT INTO sources (id, link, title, error_count, next_fetch_at) VALUES (1, 'https://e.test/feed', 'E', 0, 0)")
            .execute(&pool)
            .await
            .unwrap();

        for i in 0..5 {
            sqlx::query(
                "INSERT INTO contents (source_id, hash_id, title, created_at, updated_at) VALUES (1, ?, ?, ?, ?)",
            )
            .bind(format!("h{i}"))
            .bind(format!("title {i}"))
            .bind(format!("2020-01-0{} 00:00:00", i + 1))
            .bind(format!("2020-01-0{} 00:00:00", i + 1))
            .execute(&pool)
            .await
            .unwrap();
        }

        let deleted = repo.prune_contents(1, 1, 2).await.unwrap();
        assert_eq!(deleted, 3);
        let remaining = repo
            .existing_hash_ids(
                1,
                &["h0".into(), "h1".into(), "h2".into(), "h3".into(), "h4".into()],
            )
            .await
            .unwrap();
        assert_eq!(remaining, HashSet::from(["h3".to_owned(), "h4".to_owned()]));
    }
}
