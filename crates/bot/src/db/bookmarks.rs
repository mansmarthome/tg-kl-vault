//! Repo methods for the per-chat bookmark library. A second `impl Repo` block,
//! raw SQL with `?` binds, following the conventions in `repo.rs`.

use std::time::{SystemTime, UNIX_EPOCH};

use libsql::Value;

use super::models::{Bookmark, BookmarkTag, Content};
use super::repo::Repo;
use super::DbResult;

/// Column list matching `models::Bookmark`. Read paths bind positionally, so
/// this must stay in sync with `Bookmark::from_row`.
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
    pub async fn upsert_bookmark(&self, new: &NewBookmark<'_>) -> DbResult<UpsertOutcome> {
        let now = now_unix();
        let row = self
            .query_opt::<(i64, i64)>(
                "INSERT INTO bookmarks \
                 (chat_id, created_by, url, title, note, source_title, content_hash_id, \
                  telegraph_url, tag_state, tag_attempts, tag_next_attempt_at, notify_message_id, \
                  notify_kind, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, 0, ?, NULL, ?, ?, ?) \
                 ON CONFLICT(chat_id, url) DO UPDATE SET updated_at = excluded.updated_at \
                 RETURNING id, created_at",
                libsql::params![
                    new.chat_id,
                    new.created_by,
                    new.url,
                    new.title,
                    new.note,
                    new.source_title,
                    new.content_hash_id,
                    new.telegraph_url,
                    new.tag_next_attempt_at,
                    new.notify_kind,
                    now,
                    now,
                ],
            )
            .await?
            .ok_or(libsql::Error::QueryReturnedNoRows)?;
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
    ) -> DbResult<Option<i64>> {
        self.scalar_opt_i64(
            "UPDATE bookmarks SET notify_message_id = ?, updated_at = ? WHERE id = ? \
             RETURNING tag_state",
            libsql::params![notify_message_id, now_unix(), id],
        )
        .await
    }

    pub async fn get_bookmark(&self, chat_id: i64, id: i64) -> DbResult<Option<Bookmark>> {
        self.query_opt::<Bookmark>(
            &format!("SELECT {BOOKMARK_COLS} FROM bookmarks WHERE chat_id = ? AND id = ?"),
            libsql::params![chat_id, id],
        )
        .await
    }

    /// Unscoped lookup for the worker (which owns no chat context).
    pub async fn get_bookmark_any(&self, id: i64) -> DbResult<Option<Bookmark>> {
        self.query_opt::<Bookmark>(
            &format!("SELECT {BOOKMARK_COLS} FROM bookmarks WHERE id = ?"),
            libsql::params![id],
        )
        .await
    }

    /// Looks up a content row by hash (the 🔖 button path). May return `None`
    /// if `prune_contents` already deleted it — callers must handle expiry.
    pub async fn content_by_hash(&self, hash_id: &str) -> DbResult<Option<Content>> {
        self.query_opt::<Content>(
            "SELECT source_id, hash_id, raw_id, raw_link, title, telegraph_url, \
             created_at, updated_at FROM contents WHERE hash_id = ?",
            libsql::params![hash_id],
        )
        .await
    }

    pub async fn count_bookmarks(&self, chat_id: i64) -> DbResult<i64> {
        self.scalar_i64(
            "SELECT COUNT(*) FROM bookmarks WHERE chat_id = ?",
            libsql::params![chat_id],
        )
        .await
    }

    pub async fn bookmarks_page(
        &self,
        chat_id: i64,
        offset: i64,
        limit: i64,
    ) -> DbResult<Vec<Bookmark>> {
        self.query_all::<Bookmark>(
            &format!(
                "SELECT {BOOKMARK_COLS} FROM bookmarks WHERE chat_id = ? \
                 ORDER BY id DESC LIMIT ? OFFSET ?"
            ),
            libsql::params![chat_id, limit, offset],
        )
        .await
    }

    pub async fn count_bookmarks_by_tag(&self, chat_id: i64, tag: &str) -> DbResult<i64> {
        self.scalar_i64(
            "SELECT COUNT(*) FROM bookmarks b \
             JOIN bookmark_tags t ON t.bookmark_id = b.id \
             WHERE b.chat_id = ? AND t.tag = ?",
            libsql::params![chat_id, tag],
        )
        .await
    }

    pub async fn bookmarks_page_by_tag(
        &self,
        chat_id: i64,
        tag: &str,
        offset: i64,
        limit: i64,
    ) -> DbResult<Vec<Bookmark>> {
        // `bookmarks` and `bookmark_tags` share no column names, so the
        // unqualified `BOOKMARK_COLS` are unambiguous under the join.
        self.query_all::<Bookmark>(
            &format!(
                "SELECT {BOOKMARK_COLS} FROM bookmarks b \
                 JOIN bookmark_tags t ON t.bookmark_id = b.id \
                 WHERE b.chat_id = ? AND t.tag = ? \
                 ORDER BY b.id DESC LIMIT ? OFFSET ?"
            ),
            libsql::params![chat_id, tag, limit, offset],
        )
        .await
    }

    pub async fn count_untagged(&self, chat_id: i64) -> DbResult<i64> {
        self.scalar_i64(
            "SELECT COUNT(*) FROM bookmarks b WHERE b.chat_id = ? \
             AND NOT EXISTS (SELECT 1 FROM bookmark_tags t WHERE t.bookmark_id = b.id)",
            libsql::params![chat_id],
        )
        .await
    }

    pub async fn bookmarks_page_untagged(
        &self,
        chat_id: i64,
        offset: i64,
        limit: i64,
    ) -> DbResult<Vec<Bookmark>> {
        self.query_all::<Bookmark>(
            &format!(
                "SELECT {BOOKMARK_COLS} FROM bookmarks b WHERE b.chat_id = ? \
                 AND NOT EXISTS (SELECT 1 FROM bookmark_tags t WHERE t.bookmark_id = b.id) \
                 ORDER BY b.id DESC LIMIT ? OFFSET ?"
            ),
            libsql::params![chat_id, limit, offset],
        )
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
    ) -> DbResult<Vec<Bookmark>> {
        let pattern = like_pattern(query);
        // Numbered params so the single `pattern` bind (?2) is reused by all
        // three LIKE clauses; ?1 = chat_id, ?3 = limit.
        self.query_all::<Bookmark>(
            &format!(
                "SELECT {BOOKMARK_COLS} FROM bookmarks WHERE chat_id = ?1 \
                 AND (title LIKE ?2 ESCAPE '\\' OR url LIKE ?2 ESCAPE '\\' OR note LIKE ?2 ESCAPE '\\') \
                 ORDER BY id DESC LIMIT ?3"
            ),
            libsql::params![chat_id, pattern, limit],
        )
        .await
    }

    /// All tag rows for the given bookmark ids (used to decorate a list page).
    pub async fn tags_for_bookmarks(&self, ids: &[i64]) -> DbResult<Vec<BookmarkTag>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT bookmark_id, tag, origin FROM bookmark_tags WHERE bookmark_id IN ({placeholders}) \
             ORDER BY bookmark_id, tag"
        );
        let params: Vec<Value> = ids.iter().map(|id| Value::from(*id)).collect();
        self.query_all::<BookmarkTag>(&sql, params).await
    }

    /// `(tag, count)` pairs across the chat, for the tag index page.
    pub async fn tag_counts(&self, chat_id: i64) -> DbResult<Vec<(String, i64)>> {
        self.query_all::<(String, i64)>(
            "SELECT t.tag, COUNT(*) AS n FROM bookmark_tags t \
             JOIN bookmarks b ON b.id = t.bookmark_id \
             WHERE b.chat_id = ? GROUP BY t.tag ORDER BY t.tag",
            libsql::params![chat_id],
        )
        .await
    }

    pub async fn bookmarks_for_export(&self, chat_id: i64) -> DbResult<Vec<Bookmark>> {
        self.query_all::<Bookmark>(
            &format!("SELECT {BOOKMARK_COLS} FROM bookmarks WHERE chat_id = ? ORDER BY id DESC"),
            libsql::params![chat_id],
        )
        .await
    }

    /// Claims pending bookmarks whose retry time is due. No lease column: this
    /// is a single process with a single worker task.
    pub async fn claim_pending_bookmarks(
        &self,
        now: i64,
        limit: i64,
    ) -> DbResult<Vec<Bookmark>> {
        self.query_all::<Bookmark>(
            &format!(
                "SELECT {BOOKMARK_COLS} FROM bookmarks \
                 WHERE tag_state = 0 AND tag_next_attempt_at <= ? ORDER BY id LIMIT ?"
            ),
            libsql::params![now, limit],
        )
        .await
    }

    /// Worker success: replace all tags with the AI suggestions and mark done.
    pub async fn finish_bookmark_tagging(&self, id: i64, tags: &[String]) -> DbResult<()> {
        let tx = self.conn().transaction().await?;
        tx.execute(
            "DELETE FROM bookmark_tags WHERE bookmark_id = ?",
            libsql::params![id],
        )
        .await?;
        for tag in tags {
            tx.execute(
                "INSERT OR IGNORE INTO bookmark_tags (bookmark_id, tag, origin) VALUES (?, ?, ?)",
                libsql::params![id, tag.as_str(), ORIGIN_AI],
            )
            .await?;
        }
        tx.execute(
            "UPDATE bookmarks SET tag_state = 1, updated_at = ? WHERE id = ?",
            libsql::params![now_unix(), id],
        )
        .await?;
        tx.commit().await
    }

    /// Fills in a title fetched from page metadata (worker path).
    pub async fn set_bookmark_title(&self, id: i64, title: &str) -> DbResult<()> {
        self.exec(
            "UPDATE bookmarks SET title = ?, updated_at = ? WHERE id = ?",
            libsql::params![title, now_unix(), id],
        )
        .await?;
        Ok(())
    }

    /// Clears the notify message id (worker found the message `Gone`).
    pub async fn clear_bookmark_notify(&self, id: i64) -> DbResult<()> {
        self.exec(
            "UPDATE bookmarks SET notify_message_id = NULL, updated_at = ? WHERE id = ?",
            libsql::params![now_unix(), id],
        )
        .await?;
        Ok(())
    }

    /// Bumps the retry counter and reschedules a transient failure.
    pub async fn bump_bookmark_attempt(&self, id: i64, next_attempt_at: i64) -> DbResult<()> {
        self.exec(
            "UPDATE bookmarks SET tag_attempts = tag_attempts + 1, tag_next_attempt_at = ?, \
             updated_at = ? WHERE id = ?",
            libsql::params![next_attempt_at, now_unix(), id],
        )
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
    ) -> DbResult<bool> {
        let tx = self.conn().transaction().await?;
        let affected = tx
            .execute(
                "UPDATE bookmarks SET tag_state = 1, updated_at = ? WHERE id = ? AND chat_id = ?",
                libsql::params![now_unix(), id, chat_id],
            )
            .await?;
        if affected == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM bookmark_tags WHERE bookmark_id = ?",
            libsql::params![id],
        )
        .await?;
        for tag in tags {
            tx.execute(
                "INSERT OR IGNORE INTO bookmark_tags (bookmark_id, tag, origin) VALUES (?, ?, ?)",
                libsql::params![id, *tag, ORIGIN_MANUAL],
            )
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
    ) -> DbResult<Option<bool>> {
        let tx = self.conn().transaction().await?;
        let affected = tx
            .execute(
                "UPDATE bookmarks SET tag_state = 1, updated_at = ? WHERE id = ? AND chat_id = ?",
                libsql::params![now_unix(), id, chat_id],
            )
            .await?;
        if affected == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        let exists = {
            let mut rows = tx
                .query(
                    "SELECT 1 FROM bookmark_tags WHERE bookmark_id = ? AND tag = ?",
                    libsql::params![id, tag],
                )
                .await?;
            rows.next().await?.is_some()
        };
        let now_present = if exists {
            tx.execute(
                "DELETE FROM bookmark_tags WHERE bookmark_id = ? AND tag = ?",
                libsql::params![id, tag],
            )
            .await?;
            false
        } else {
            tx.execute(
                "INSERT INTO bookmark_tags (bookmark_id, tag, origin) VALUES (?, ?, ?)",
                libsql::params![id, tag, ORIGIN_MANUAL],
            )
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
    ) -> DbResult<bool> {
        let affected = self
            .exec(
                "UPDATE bookmarks SET note = ?, updated_at = ? WHERE id = ? AND chat_id = ?",
                libsql::params![note, now_unix(), id, chat_id],
            )
            .await?;
        Ok(affected > 0)
    }

    /// Deletes a bookmark and its tag rows in one transaction (no FK cascade;
    /// see migration 0004). Scoped by `chat_id`.
    pub async fn delete_bookmark(&self, chat_id: i64, id: i64) -> DbResult<bool> {
        let tx = self.conn().transaction().await?;
        let affected = tx
            .execute(
                "DELETE FROM bookmarks WHERE chat_id = ? AND id = ?",
                libsql::params![chat_id, id],
            )
            .await?;
        if affected == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM bookmark_tags WHERE bookmark_id = ?",
            libsql::params![id],
        )
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
        let db = db::connect(db_path.to_str().unwrap()).await.unwrap();
        // Leak the tempdir so the file outlives the test body.
        std::mem::forget(dir);
        Repo::new(db)
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
