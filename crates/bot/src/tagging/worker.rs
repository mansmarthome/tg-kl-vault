//! Background tagging worker. Every 5s it claims a small batch of pending
//! bookmarks, fills in a title from page metadata when missing, tags them, and
//! edits the notification message.
//!
//! Order matters: **commit tags first, then edit the message.** If the edit
//! fails the tags have already landed and the row is terminal; the other way
//! round, a crash would re-tag and burn quota. The edit is **never retried** —
//! no second state machine.

use std::time::Duration;

use reqwest::Client;
use tokio::sync::watch;
use tracing::warn;

use crate::bookmark::render;
use crate::bot::bookmarks;
use crate::bot::runtime::chat_lang;
use crate::bot::sender::{EditOptions, EditOutcome, MessageEditor};
use crate::config::{Config, MessageMode};
use crate::db::bookmarks::now_unix;
use crate::db::models::Bookmark;
use crate::db::repo::Repo;

use super::heuristic::HeuristicTagger;
use super::{metadata, quota, TagInput, Tagger};

const BATCH: i64 = 3;
const POLL_SECS: u64 = 5;

pub struct TagWorker<T: Tagger, E: MessageEditor> {
    repo: Repo,
    tagger: T,
    heuristic: HeuristicTagger,
    editor: E,
    config: Config,
    http: Client,
    /// Meter the daily Gemini quota (true only when the primary tagger is
    /// Gemini). The heuristic is unlimited.
    meter_quota: bool,
}

impl<T: Tagger, E: MessageEditor> TagWorker<T, E> {
    pub fn new(repo: Repo, tagger: T, editor: E, config: Config, http: Client, meter_quota: bool) -> Self {
        let heuristic = HeuristicTagger::new(config.bookmark.ai.max_tags as usize);
        Self { repo, tagger, heuristic, editor, config, http, meter_quota }
    }

    pub async fn run_until_shutdown(&self, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
        loop {
            if let Err(err) = self.run_once().await {
                warn!(error = %err, "tag worker pass failed");
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

    pub async fn run_once(&self) -> anyhow::Result<()> {
        let now = now_unix();
        for bm in self.repo.claim_pending_bookmarks(now, BATCH).await? {
            self.process(&bm).await?;
        }
        Ok(())
    }

    async fn process(&self, bm: &Bookmark) -> anyhow::Result<()> {
        let now = now_unix();

        // 1. Fill in a title from page metadata when the client gave us none.
        let mut title = bm.title.clone();
        let mut excerpt = String::new();
        if title.is_empty() {
            if let Ok(md) = metadata::fetch_metadata(&self.http, &self.config.user_agent, &bm.url).await {
                if let Some(t) = md.title {
                    self.repo.set_bookmark_title(bm.id, &t).await?;
                    title = t;
                }
                if let Some(d) = md.description {
                    excerpt = d;
                }
            }
        }

        // 2. Per-chat AI toggle: off → no automatic tags (still terminal).
        let ai_on = self
            .repo
            .get_option(&bookmarks::ai_option_key(bm.chat_id))
            .await?
            .as_deref()
            != Some("0");

        // 3. Decide tags. The heuristic is the terminal fallback and can't fail.
        let tags: Vec<String> = if !ai_on {
            Vec::new()
        } else {
            let input = TagInput { title: &title, url: &bm.url, excerpt: &excerpt };
            let over_quota = self.meter_quota
                && !quota::try_consume(&self.repo, self.config.bookmark.ai.daily_quota)
                    .await
                    .unwrap_or(true);
            if over_quota {
                self.heuristic.classify(&input)
            } else {
                match self.tagger.suggest(&input).await {
                    Ok(tags) if !tags.is_empty() => tags,
                    // Empty / all-invalid → heuristic; never leave pending.
                    Ok(_) => self.heuristic.classify(&input),
                    Err(err) => {
                        // Transient: bounded retry ladder, then heuristic.
                        if bm.tag_attempts >= 2 {
                            warn!(id = bm.id, error = %err, "tagger failed 3x; using heuristic");
                            self.heuristic.classify(&input)
                        } else {
                            let delay = if bm.tag_attempts == 0 { 30 } else { 120 };
                            self.repo.bump_bookmark_attempt(bm.id, now + delay).await?;
                            return Ok(());
                        }
                    }
                }
            }
        };

        // 4. Commit tags (terminal) BEFORE editing.
        self.repo.finish_bookmark_tagging(bm.id, &tags).await?;

        // 5. Re-read the notify id; skip the edit if there's nothing to edit.
        let Some(fresh) = self.repo.get_bookmark_any(bm.id).await? else {
            return Ok(());
        };
        let Some(message_id) = fresh.notify_message_id else {
            return Ok(());
        };
        let message_id = message_id as i32;
        let lang = chat_lang(&self.repo, fresh.chat_id).await;

        // 6. Edit — never retried.
        let outcome = if fresh.notify_kind == 1 {
            // Relabel the 🔖 button with the tags, keeping the 📝 summary
            // button (keyed by the content hash) if it's enabled for this chat.
            let label = render::button_tag_label(&tags, lang.bm_saved_button());
            let sum = bookmarks::summary_enabled_raw(&self.repo, &self.config, fresh.chat_id)
                .await
                .then_some(fresh.content_hash_id.as_deref())
                .flatten();
            let markup = bookmarks::item_keyboard(
                Some(bookmarks::BmBtn::Saved { id: fresh.id, label }),
                sum,
            )
            .unwrap_or_default();
            self.editor.edit_markup(fresh.chat_id, message_id, markup).await
        } else {
            let text = render::render_detail(&fresh, &tags, lang);
            let options = EditOptions {
                parse_mode: MessageMode::Html,
                disable_web_page_preview: true,
                reply_markup: Some(bookmarks::detail_markup(fresh.id, lang)),
            };
            self.editor.edit_text(fresh.chat_id, message_id, &text, options).await
        };

        match outcome {
            Ok(EditOutcome::Gone) => self.repo.clear_bookmark_notify(fresh.id).await?,
            Ok(_) => {}
            Err(err) => warn!(id = fresh.id, error = %err, "tag worker edit failed (no retry)"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::sender::test_support::RecordingEditor;
    use crate::config::Config;
    use crate::db::{self, bookmarks::NewBookmark};
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn repo() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::connect(dir.path().join("w.db").to_str().unwrap()).await.unwrap();
        std::mem::forget(dir);
        Repo::new(pool)
    }

    fn worker<T: Tagger>(
        repo: Repo,
        tagger: T,
        editor: RecordingEditor,
    ) -> TagWorker<T, RecordingEditor> {
        TagWorker::new(repo, tagger, editor, Config::default(), Client::new(), false)
    }

    /// Tagger returning a fixed set; counts calls.
    struct StubTagger {
        tags: Vec<String>,
        calls: AtomicUsize,
    }
    impl Tagger for StubTagger {
        async fn suggest(&self, _: &TagInput<'_>) -> anyhow::Result<Vec<String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.tags.clone())
        }
    }

    struct FailingTagger {
        calls: AtomicUsize,
    }
    impl Tagger for FailingTagger {
        async fn suggest(&self, _: &TagInput<'_>) -> anyhow::Result<Vec<String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::anyhow!("boom"))
        }
    }

    async fn insert(repo: &Repo, notify_msg: Option<i64>, notify_kind: i64) -> i64 {
        let id = repo
            .upsert_bookmark(&NewBookmark {
                chat_id: 100,
                created_by: 100,
                url: "https://x.test/a",
                title: "A Rust story", // non-empty → no metadata fetch
                note: "",
                source_title: "",
                content_hash_id: None,
                telegraph_url: None,
                notify_kind,
                tag_next_attempt_at: 0,
            })
            .await
            .unwrap()
            .id;
        if let Some(mid) = notify_msg {
            repo.set_bookmark_notify(id, mid).await.unwrap();
        }
        id
    }

    async fn reset_due(repo: &Repo, id: i64) {
        sqlx::query("UPDATE bookmarks SET tag_next_attempt_at = 0 WHERE id = ?")
            .bind(id)
            .execute(repo.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn pending_gets_tagged_and_edited() {
        let repo = repo().await;
        let id = insert(&repo, Some(555), 0).await;
        let w = worker(repo.clone(), StubTagger { tags: vec!["tech".into()], calls: AtomicUsize::new(0) }, RecordingEditor::default());
        w.run_once().await.unwrap();

        let bm = repo.get_bookmark(100, id).await.unwrap().unwrap();
        assert_eq!(bm.tag_state, 1);
        assert_eq!(repo.tags_for_bookmarks(&[id]).await.unwrap().len(), 1);
        assert_eq!(w.editor.edits.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn gone_message_clears_notify_but_row_is_terminal() {
        let repo = repo().await;
        let id = insert(&repo, Some(777), 0).await;
        let editor = RecordingEditor { gone_message_ids: vec![777], ..Default::default() };
        let w = worker(repo.clone(), StubTagger { tags: vec!["tech".into()], calls: AtomicUsize::new(0) }, editor);
        w.run_once().await.unwrap();

        let bm = repo.get_bookmark(100, id).await.unwrap().unwrap();
        assert_eq!(bm.tag_state, 1);
        assert_eq!(bm.notify_message_id, None, "Gone must clear notify id");
    }

    #[tokio::test]
    async fn empty_suggestion_finalizes_via_heuristic() {
        let repo = repo().await;
        let id = insert(&repo, Some(1), 0).await;
        let w = worker(repo.clone(), StubTagger { tags: vec![], calls: AtomicUsize::new(0) }, RecordingEditor::default());
        w.run_once().await.unwrap();
        let bm = repo.get_bookmark(100, id).await.unwrap().unwrap();
        assert_eq!(bm.tag_state, 1, "must finalize, not stay pending");
        assert!(!repo.tags_for_bookmarks(&[id]).await.unwrap().is_empty(), "heuristic yields >=1 tag");
    }

    #[tokio::test]
    async fn transient_failures_stop_after_exactly_three_attempts() {
        let repo = repo().await;
        let id = insert(&repo, Some(1), 0).await;
        let w = worker(repo.clone(), FailingTagger { calls: AtomicUsize::new(0) }, RecordingEditor::default());

        w.run_once().await.unwrap(); // attempt 0 -> bump to 1
        reset_due(&repo, id).await;
        w.run_once().await.unwrap(); // attempt 1 -> bump to 2
        reset_due(&repo, id).await;
        w.run_once().await.unwrap(); // attempt 2 -> heuristic finalize

        let bm = repo.get_bookmark(100, id).await.unwrap().unwrap();
        assert_eq!(bm.tag_state, 1);
        assert_eq!(w.tagger.calls.load(Ordering::SeqCst), 3, "exactly 3 suggest() calls");

        // A further pass finds nothing pending (idempotent).
        reset_due(&repo, id).await;
        w.run_once().await.unwrap();
        assert_eq!(w.tagger.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn null_notify_tags_without_editing() {
        let repo = repo().await;
        let id = insert(&repo, None, 0).await;
        let w = worker(repo.clone(), StubTagger { tags: vec!["ai".into()], calls: AtomicUsize::new(0) }, RecordingEditor::default());
        w.run_once().await.unwrap();
        let bm = repo.get_bookmark(100, id).await.unwrap().unwrap();
        assert_eq!(bm.tag_state, 1);
        assert_eq!(w.editor.edits.lock().unwrap().len(), 0, "no edit when notify id is NULL");
    }

    #[tokio::test]
    async fn second_pass_is_idempotent() {
        let repo = repo().await;
        insert(&repo, Some(1), 0).await;
        let w = worker(repo.clone(), StubTagger { tags: vec!["tech".into()], calls: AtomicUsize::new(0) }, RecordingEditor::default());
        w.run_once().await.unwrap();
        w.run_once().await.unwrap();
        assert_eq!(w.tagger.calls.load(Ordering::SeqCst), 1, "restart must not re-tag");
    }
}
