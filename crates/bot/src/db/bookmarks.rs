//! Repo methods for the per-chat bookmark library. A second `impl Repo` block
//! (allowed: `Repo::pool()` is already public), raw SQL with `?` binds,
//! following the conventions in `repo.rs`.

use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::{AssertSqlSafe, QueryBuilder, Sqlite};

use super::models::{Bookmark, BookmarkTag, Content};
use super::repo::Repo;

/// sqlx 0.9 only accepts `&'static str` SQL by default. These query strings
/// interpolate `BOOKMARK_COLS` (a compile-time const) and nothing else — no
/// caller input reaches the SQL text — so asserting them safe is sound. All
/// runtime values go through `?` binds.
fn safe(sql: String) -> AssertSqlSafe<String> {
    AssertSqlSafe(sql)
}

/// Column list matching `models::Bookmark`. `FromRow` binds by name, but being
/// explicit keeps the read path stable if the table grows.
const BOOKMARK_COLS: &str = "id, chat_id, created_by, url, title, note, source_title, \
     content_hash_id, telegraph_url, tag_state, tag_attempts, tag_next_attempt_at, \
     notify_message_id, notify_kind, created_at, updated_at";

/// Tag origin stored in `bookmark_tags.origin`.
pub const ORIGIN_AI: i64 = 0;
pub const ORIGIN_MANUAL: i64 = 1;

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Fields for a new bookmark insert. A struct keeps the upsert under clippy's
/// `too_many_arguments` threshold and reads clearly at call sites.
pub struct NewBookmark<'a> {
    pub chat_id: i64,
    pub created_by: i64,
    pub url: &'a str,
    pub title: &'a str,
    pub note: &'a str,
    pub source_title: &'a str,
    pub content_hash_id: Option<&'a str>,
    pub telegraph_url: Option<&'a str>,
    pub notify_kind: i64,
    /// First tagging attempt scheduled at this unix time. Handlers set
    /// `now + 3` so the worker doesn't race the insert→reply handshake.
    pub tag_next_attempt_at: i64,
}

/// Outcome of `upsert_bookmark`.
pub struct UpsertOutcome {
    pub id: i64,
    /// True when the (chat_id, url) row already existed before this call.
    pub existed: bool,
}

/// Escapes `LIKE` metacharacters so a query like `100%` is a literal search,
/// not "match everything". Paired with `ESCAPE '\'` in the SQL.
fn like_pattern(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len() + 2);
    escaped.push('%');
    for ch in query.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('%');
    escaped
}

impl Repo {
    /// Inserts a bookmark, or returns the existing (chat_id, url) row.
    ///
    /// Uses `RETURNING id, created_at` rather than `last_insert_rowid()`:
    /// SQLite only updates that on a real insert, so the `ON CONFLICT … DO
    /// UPDATE` branch would hand back a *stale* id and the caller would bind a
    /// keyboard to someone else's bookmark. `existed` is derived by comparing
    /// the returned `created_at` against `now`.
    pub async fn upsert_bookmark(&self, new: &NewBookmark<'_>) -> sqlx::Result<UpsertOutcome> {
        let now = now_unix();
        let row: (i64, i64) = sqlx::query_as(
            "INSERT INTO bookmarks \
             (chat_id, created_by, url, title, note, source_title, content_hash_id, \
              telegraph_url, tag_state, tag_attempts, tag_next_attempt_at, notify_message_id, \
              notify_kind, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, 0, ?, NULL, ?, ?, ?) \
             ON CONFLICT(chat_id, url) DO UPDATE SET updated_at = excluded.updated_at \
             RETURNING id, created_at",
        )
        .bind(new.chat_id)
        .bind(new.created_by)
        .bind(new.url)
        .bind(new.title)
        .bind(new.note)
        .bind(new.source_title)
        .bind(new.content_hash_id)
        .bind(new.telegraph_url)
        .bind(new.tag_next_attempt_at)
        .bind(new.notify_kind)
        .bind(now)
        .bind(now)
        .fetch_one(self.pool())
        .await?;
        Ok(UpsertOutcome {
            id: row.0,
            existed: row.1 != now,
        })
    }

    /// Records the notification message the worker should later edit, and
    /// returns the current `tag_state` so the handler can detect a worker that
    /// already finished (in which case it renders the final text itself).
    pub async fn set_bookmark_notify(
        &self,
        id: i64,
        notify_message_id: i64,
    ) -> sqlx::Result<Option<i64>> {
        sqlx::query_scalar(
            "UPDATE bookmarks SET notify_message_id = ?, updated_at = ? WHERE id = ? \
             RETURNING tag_state",
        )
        .bind(notify_message_id)
        .bind(now_unix())
        .bind(id)
        .fetch_optional(self.pool())
        .await
    }

    pub async fn get_bookmark(&self, chat_id: i64, id: i64) -> sqlx::Result<Option<Bookmark>> {
        sqlx::query_as::<_, Bookmark>(safe(format!(
            "SELECT {BOOKMARK_COLS} FROM bookmarks WHERE chat_id = ? AND id = ?"
        )))
        .bind(chat_id)
        .bind(id)
        .fetch_optional(self.pool())
        .await
    }

    /// Unscoped lookup for the worker (which owns no chat context).
    pub async fn get_bookmark_any(&self, id: i64) -> sqlx::Result<Option<Bookmark>> {
        sqlx::query_as::<_, Bookmark>(safe(format!(
            "SELECT {BOOKMARK_COLS} FROM bookmarks WHERE id = ?"
        )))
        .bind(id)
        .fetch_optional(self.pool())
        .await
    }

    /// Looks up a content row by hash (the 🔖 button path). May return `None`
    /// if `prune_contents` already deleted it — callers must handle expiry.
    pub async fn content_by_hash(&self, hash_id: &str) -> sqlx::Result<Option<Content>> {
        sqlx::query_as::<_, Content>(
            "SELECT source_id, hash_id, raw_id, raw_link, title, telegraph_url, \
             created_at, updated_at FROM contents WHERE hash_id = ?",
        )
        .bind(hash_id)
        .fetch_optional(self.pool())
        .await
    }

    pub async fn count_bookmarks(&self, chat_id: i64) -> sqlx::Result<i64> {
        sqlx::query_scalar("SELECT COUNT(*) FROM bookmarks WHERE chat_id = ?")
            .bind(chat_id)
            .fetch_one(self.pool())
            .await
    }

    pub async fn bookmarks_page(
        &self,
        chat_id: i64,
        offset: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<Bookmark>> {
        sqlx::query_as::<_, Bookmark>(safe(format!(
            "SELECT {BOOKMARK_COLS} FROM bookmarks WHERE chat_id = ? \
             ORDER BY id DESC LIMIT ? OFFSET ?"
        )))
        .bind(chat_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(self.pool())
        .await
    }

    pub async fn count_bookmarks_by_tag(&self, chat_id: i64, tag: &str) -> sqlx::Result<i64> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM bookmarks b \
             JOIN bookmark_tags t ON t.bookmark_id = b.id \
             WHERE b.chat_id = ? AND t.tag = ?",
        )
        .bind(chat_id)
        .bind(tag)
        .fetch_one(self.pool())
        .await
    }

    pub async fn bookmarks_page_by_tag(
        &self,
        chat_id: i64,
        tag: &str,
        offset: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<Bookmark>> {
        // `bookmarks` and `bookmark_tags` share no column names, so the
        // unqualified `BOOKMARK_COLS` are unambiguous under the join.
        sqlx::query_as::<_, Bookmark>(safe(format!(
            "SELECT {BOOKMARK_COLS} FROM bookmarks b \
             JOIN bookmark_tags t ON t.bookmark_id = b.id \
             WHERE b.chat_id = ? AND t.tag = ? \
             ORDER BY b.id DESC LIMIT ? OFFSET ?"
        )))
        .bind(chat_id)
        .bind(tag)
        .bind(limit)
        .bind(offset)
        .fetch_all(self.pool())
        .await
    }

    pub async fn count_untagged(&self, chat_id: i64) -> sqlx::Result<i64> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM bookmarks b WHERE b.chat_id = ? \
             AND NOT EXISTS (SELECT 1 FROM bookmark_tags t WHERE t.bookmark_id = b.id)",
        )
        .bind(chat_id)
        .fetch_one(self.pool())
        .await
    }

    pub async fn bookmarks_page_untagged(
        &self,
        chat_id: i64,
        offset: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<Bookmark>> {
        sqlx::query_as::<_, Bookmark>(safe(format!(
            "SELECT {BOOKMARK_COLS} FROM bookmarks b WHERE b.chat_id = ? \
             AND NOT EXISTS (SELECT 1 FROM bookmark_tags t WHERE t.bookmark_id = b.id) \
             ORDER BY b.id DESC LIMIT ? OFFSET ?"
        )))
        .bind(chat_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(self.pool())
        .await
    }

    /// Keyword search over title/url/note. `LIKE` with an escaped pattern +
    /// `ESCAPE '\'`; SQLite `LIKE` is already ASCII case-insensitive, so no
    /// `COLLATE`/`lower()` (which wouldn't help CJK anyway).
    pub async fn search_bookmarks(
        &self,
        chat_id: i64,
        query: &str,
        limit: i64,
    ) -> sqlx::Result<Vec<Bookmark>> {
        let pattern = like_pattern(query);
        // Numbered params so the single `pattern` bind (?2) is reused by all
        // three LIKE clauses; ?1 = chat_id, ?3 = limit.
        sqlx::query_as::<_, Bookmark>(safe(format!(
            "SELECT {BOOKMARK_COLS} FROM bookmarks WHERE chat_id = ?1 \
             AND (title LIKE ?2 ESCAPE '\\' OR url LIKE ?2 ESCAPE '\\' OR note LIKE ?2 ESCAPE '\\') \
             ORDER BY id DESC LIMIT ?3"
        )))
        .bind(chat_id)
        .bind(pattern)
        .bind(limit)
        .fetch_all(self.pool())
        .await
    }

    /// All tag rows for the given bookmark ids (used to decorate a list page).
    pub async fn tags_for_bookmarks(&self, ids: &[i64]) -> sqlx::Result<Vec<BookmarkTag>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT bookmark_id, tag, origin FROM bookmark_tags WHERE bookmark_id IN (",
        );
        let mut separated = builder.separated(",");
        for id in ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(") ORDER BY bookmark_id, tag");
        builder
            .build_query_as::<BookmarkTag>()
            .fetch_all(self.pool())
            .await
    }

    /// `(tag, count)` pairs across the chat, for the tag index page.
    pub async fn tag_counts(&self, chat_id: i64) -> sqlx::Result<Vec<(String, i64)>> {
        sqlx::query_as::<_, (String, i64)>(
            "SELECT t.tag, COUNT(*) AS n FROM bookmark_tags t \
             JOIN bookmarks b ON b.id = t.bookmark_id \
             WHERE b.chat_id = ? GROUP BY t.tag ORDER BY t.tag",
        )
        .bind(chat_id)
        .fetch_all(self.pool())
        .await
    }

    pub async fn bookmarks_for_export(&self, chat_id: i64) -> sqlx::Result<Vec<Bookmark>> {
        sqlx::query_as::<_, Bookmark>(safe(format!(
            "SELECT {BOOKMARK_COLS} FROM bookmarks WHERE chat_id = ? ORDER BY id DESC"
        )))
        .bind(chat_id)
        .fetch_all(self.pool())
        .await
    }

    /// Claims pending bookmarks whose retry time is due. No lease column: this
    /// is a single process with a single worker task.
    pub async fn claim_pending_bookmarks(
        &self,
        now: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<Bookmark>> {
        sqlx::query_as::<_, Bookmark>(safe(format!(
            "SELECT {BOOKMARK_COLS} FROM bookmarks \
             WHERE tag_state = 0 AND tag_next_attempt_at <= ? ORDER BY id LIMIT ?"
        )))
        .bind(now)
        .bind(limit)
        .fetch_all(self.pool())
        .await
    }

    /// Worker success: replace all tags with the AI suggestions and mark done.
    pub async fn finish_bookmark_tagging(&self, id: i64, tags: &[String]) -> sqlx::Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("DELETE FROM bookmark_tags WHERE bookmark_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for tag in tags {
            sqlx::query(
                "INSERT OR IGNORE INTO bookmark_tags (bookmark_id, tag, origin) VALUES (?, ?, ?)",
            )
            .bind(id)
            .bind(tag)
            .bind(ORIGIN_AI)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE bookmarks SET tag_state = 1, updated_at = ? WHERE id = ?")
            .bind(now_unix())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }

    /// Fills in a title fetched from page metadata (worker path).
    pub async fn set_bookmark_title(&self, id: i64, title: &str) -> sqlx::Result<()> {
        sqlx::query("UPDATE bookmarks SET title = ?, updated_at = ? WHERE id = ?")
            .bind(title)
            .bind(now_unix())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Clears the notify message id (worker found the message `Gone`).
    pub async fn clear_bookmark_notify(&self, id: i64) -> sqlx::Result<()> {
        sqlx::query("UPDATE bookmarks SET notify_message_id = NULL, updated_at = ? WHERE id = ?")
            .bind(now_unix())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Bumps the retry counter and reschedules a transient failure.
    pub async fn bump_bookmark_attempt(&self, id: i64, next_attempt_at: i64) -> sqlx::Result<()> {
        sqlx::query(
            "UPDATE bookmarks SET tag_attempts = tag_attempts + 1, tag_next_attempt_at = ?, \
             updated_at = ? WHERE id = ?",
        )
        .bind(next_attempt_at)
        .bind(now_unix())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Manual tag set (`/bmtag`): replaces all tags, origin=manual, marks done.
    /// Manual sets `tag_state = 1`, and since `claim` only reads `tag_state = 0`
    /// "AI never overwrites a manually-tagged bookmark" falls out naturally.
    pub async fn set_bookmark_tags_manual(
        &self,
        chat_id: i64,
        id: i64,
        tags: &[&str],
    ) -> sqlx::Result<bool> {
        let mut tx = self.pool().begin().await?;
        let res = sqlx::query(
            "UPDATE bookmarks SET tag_state = 1, updated_at = ? WHERE id = ? AND chat_id = ?",
        )
        .bind(now_unix())
        .bind(id)
        .bind(chat_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query("DELETE FROM bookmark_tags WHERE bookmark_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for tag in tags {
            sqlx::query(
                "INSERT OR IGNORE INTO bookmark_tags (bookmark_id, tag, origin) VALUES (?, ?, ?)",
            )
            .bind(id)
            .bind(*tag)
            .bind(ORIGIN_MANUAL)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Toggles a single tag (the tag-toggle keyboard). Returns whether the tag
    /// is present *after* the toggle. Marks the bookmark done (manual origin).
    pub async fn toggle_bookmark_tag(
        &self,
        chat_id: i64,
        id: i64,
        tag: &str,
    ) -> sqlx::Result<Option<bool>> {
        let mut tx = self.pool().begin().await?;
        let res = sqlx::query(
            "UPDATE bookmarks SET tag_state = 1, updated_at = ? WHERE id = ? AND chat_id = ?",
        )
        .bind(now_unix())
        .bind(id)
        .bind(chat_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM bookmark_tags WHERE bookmark_id = ? AND tag = ?")
                .bind(id)
                .bind(tag)
                .fetch_optional(&mut *tx)
                .await?;
        let now_present = if existing.is_some() {
            sqlx::query("DELETE FROM bookmark_tags WHERE bookmark_id = ? AND tag = ?")
                .bind(id)
                .bind(tag)
                .execute(&mut *tx)
                .await?;
            false
        } else {
            sqlx::query(
                "INSERT INTO bookmark_tags (bookmark_id, tag, origin) VALUES (?, ?, ?)",
            )
            .bind(id)
            .bind(tag)
            .bind(ORIGIN_MANUAL)
            .execute(&mut *tx)
            .await?;
            true
        };
        tx.commit().await?;
        Ok(Some(now_present))
    }

    pub async fn set_bookmark_note(
        &self,
        chat_id: i64,
        id: i64,
        note: &str,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query(
            "UPDATE bookmarks SET note = ?, updated_at = ? WHERE id = ? AND chat_id = ?",
        )
        .bind(note)
        .bind(now_unix())
        .bind(id)
        .bind(chat_id)
        .execute(self.pool())
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Deletes a bookmark and its tag rows in one transaction (no FK cascade;
    /// see migration 0004). Scoped by `chat_id`.
    pub async fn delete_bookmark(&self, chat_id: i64, id: i64) -> sqlx::Result<bool> {
        let mut tx = self.pool().begin().await?;
        let res = sqlx::query("DELETE FROM bookmarks WHERE chat_id = ? AND id = ?")
            .bind(chat_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query("DELETE FROM bookmark_tags WHERE bookmark_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn test_repo() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let pool = db::connect(db_path.to_str().unwrap()).await.unwrap();
        // Leak the tempdir so the file outlives the test body.
        std::mem::forget(dir);
        Repo::new(pool)
    }

    fn sample(chat_id: i64, url: &str, title: &str) -> NewBookmark<'static> {
        // Intentionally leaks small strings for test brevity.
        NewBookmark {
            chat_id,
            created_by: chat_id,
            url: Box::leak(url.to_owned().into_boxed_str()),
            title: Box::leak(title.to_owned().into_boxed_str()),
            note: "",
            source_title: "",
            content_hash_id: None,
            telegraph_url: None,
            notify_kind: 0,
            tag_next_attempt_at: 0,
        }
    }

    #[tokio::test]
    async fn upsert_returns_same_id_on_second_call() {
        let repo = test_repo().await;
        let first = repo.upsert_bookmark(&sample(1, "https://a.test/x", "A")).await.unwrap();
        assert!(!first.existed);
        let second = repo.upsert_bookmark(&sample(1, "https://a.test/x", "A")).await.unwrap();
        assert_eq!(first.id, second.id, "conflict branch must return the same id");
    }

    #[tokio::test]
    async fn reads_are_chat_isolated() {
        let repo = test_repo().await;
        repo.upsert_bookmark(&sample(1, "https://a.test/x", "A")).await.unwrap();
        repo.upsert_bookmark(&sample(2, "https://a.test/x", "A")).await.unwrap();
        assert_eq!(repo.count_bookmarks(1).await.unwrap(), 1);
        assert_eq!(repo.count_bookmarks(2).await.unwrap(), 1);
        let page = repo.bookmarks_page(1, 0, 10).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].chat_id, 1);
    }

    #[tokio::test]
    async fn search_escapes_like_metacharacters() {
        let repo = test_repo().await;
        repo.upsert_bookmark(&sample(1, "https://a.test/pure", "100% pure")).await.unwrap();
        repo.upsert_bookmark(&sample(1, "https://a.test/other", "plain")).await.unwrap();

        let hits = repo.search_bookmarks(1, "100%", 10).await.unwrap();
        assert_eq!(hits.len(), 1, "'100%' should match the literal title only");
        assert_eq!(hits[0].title, "100% pure");

        // A lone `%` is escaped to a literal, so it matches nothing (no title
        // contains a literal percent besides the one we already found).
        let all = repo.search_bookmarks(1, "%", 10).await.unwrap();
        assert_eq!(all.len(), 1, "'%' must not behave as match-all");
    }

    #[tokio::test]
    async fn claim_respects_next_attempt_and_limit() {
        let repo = test_repo().await;
        for i in 0..5 {
            let mut nb = sample(1, Box::leak(format!("https://a.test/{i}").into_boxed_str()), "t");
            nb.tag_next_attempt_at = if i < 3 { 100 } else { 10_000 };
            repo.upsert_bookmark(&nb).await.unwrap();
        }
        // now = 200: only the 3 with next_attempt <= 200 are eligible; limit 2.
        let claimed = repo.claim_pending_bookmarks(200, 2).await.unwrap();
        assert_eq!(claimed.len(), 2);
        let claimed_all = repo.claim_pending_bookmarks(200, 10).await.unwrap();
        assert_eq!(claimed_all.len(), 3, "future-dated rows must not be claimed");
    }

    #[tokio::test]
    async fn finish_tagging_and_delete_cascade_tags() {
        let repo = test_repo().await;
        let bm = repo.upsert_bookmark(&sample(1, "https://a.test/x", "A")).await.unwrap();
        repo.finish_bookmark_tagging(bm.id, &["tech".to_owned(), "ai".to_owned()])
            .await
            .unwrap();

        let tags = repo.tags_for_bookmarks(&[bm.id]).await.unwrap();
        assert_eq!(tags.len(), 2);
        let stored = repo.get_bookmark(1, bm.id).await.unwrap().unwrap();
        assert_eq!(stored.tag_state, 1);

        assert!(repo.delete_bookmark(1, bm.id).await.unwrap());
        assert!(repo.tags_for_bookmarks(&[bm.id]).await.unwrap().is_empty());
        assert!(repo.get_bookmark(1, bm.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn toggle_tag_flips_presence() {
        let repo = test_repo().await;
        let bm = repo.upsert_bookmark(&sample(1, "https://a.test/x", "A")).await.unwrap();
        assert_eq!(repo.toggle_bookmark_tag(1, bm.id, "tech").await.unwrap(), Some(true));
        assert_eq!(repo.toggle_bookmark_tag(1, bm.id, "tech").await.unwrap(), Some(false));
        // Wrong chat cannot toggle.
        assert_eq!(repo.toggle_bookmark_tag(2, bm.id, "tech").await.unwrap(), None);
    }
}
