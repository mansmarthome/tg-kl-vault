use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tracing::{info, warn};

use crate::{
    bot::{
        render::{render_html, render_markdown, MessageData},
        sender::{MessageSender, SendOptions, SendOutcome},
    },
    config::{Config, MessageMode},
    db::{
        models::{Content, Source, Subscribe},
        repo::Repo,
    },
    feed::{fetch::{FetchOutcome, Fetcher}, hash::gen_hash_id, parse::{parse_feed, ParsedItem}},
    preview::{trim_description, PublishRequest, PreviewPublisher},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerOptions {
    pub dry_run: bool,
    pub batch_limit: i64,
}

impl Default for SchedulerOptions {
    fn default() -> Self {
        Self { dry_run: false, batch_limit: 50 }
    }
}

pub struct Scheduler<P, S> {
    repo: Repo,
    fetcher: Fetcher,
    publisher: P,
    sender: S,
    config: Config,
    options: SchedulerOptions,
}

impl<P, S> Scheduler<P, S>
where
    P: PreviewPublisher,
    S: MessageSender,
{
    pub fn new(
        repo: Repo,
        fetcher: Fetcher,
        publisher: P,
        sender: S,
        config: Config,
        options: SchedulerOptions,
    ) -> Self {
        Self { repo, fetcher, publisher, sender, config, options }
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    pub async fn run_until_shutdown(&self, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
        loop {
            self.run_once().await?;
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_ok() && *shutdown.borrow() {
                        break;
                    }
                }
                () = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
        }
        Ok(())
    }

    /// Run one bounded due-source pass. Keeping this separable makes dry-run
    /// testing safe and enables the required production DB dry-run gate.
    pub async fn run_once(&self) -> anyhow::Result<()> {
        let now = now_unix();
        let due = self.repo.sources_due(now, self.options.batch_limit).await?;
        for source in due {
            let Some(link) = source.link.as_deref().filter(|s| !s.is_empty()) else {
                continue;
            };

            match self.fetcher.fetch(link, source.etag.as_deref(), source.last_modified.as_deref()).await {
                Ok(FetchOutcome::Unchanged) => {
                    let next = next_fetch_at(now, self.config.update_interval);
                    if !self.options.dry_run {
                        self.repo.mark_source_success(source.id, None, None, next).await?;
                    }
                }
                Ok(FetchOutcome::Modified(feed)) => {
                    let parsed = parse_feed(&feed.body)?;
                    let hashes = parsed
                        .items
                        .iter()
                        .map(|item| gen_hash_id(link, &item.guid))
                        .collect::<Vec<_>>();
                    let existing = self.repo.existing_hash_ids(source.id, &hashes).await?;

                    // Fetched once per source, matching Go's BroadcastNews,
                    // which loads subscribers before looping new contents.
                    let subs = if self.options.dry_run {
                        Vec::new()
                    } else {
                        self.repo.subscribes_for_source(source.id).await?
                    };

                    for (item, hash_id) in parsed.items.iter().zip(hashes) {
                        if existing.contains(&hash_id) {
                            continue;
                        }
                        info!(
                            chat_id = tracing::field::Empty,
                            source_id = source.id,
                            hash_id = %hash_id,
                            title = %item.title,
                            dry_run = self.options.dry_run,
                            "would send"
                        );
                        if self.options.dry_run {
                            continue;
                        }

                        let telegraph_url = self
                            .publisher
                            .publish(&PublishRequest {
                                title: &item.title,
                                author_name: Some(&self.config.telegraph_author_name),
                                author_url: non_empty(&self.config.telegraph_author_url),
                                html: item.content.as_deref().or(item.description.as_deref()).unwrap_or(""),
                            })
                            .await
                            .unwrap_or_else(|err| {
                                warn!(source_id = source.id, %hash_id, error = %err, "telegraph publish failed");
                                None
                            });

                        self.repo
                            .insert_content(&Content {
                                source_id: Some(source.id),
                                hash_id: hash_id.clone(),
                                raw_id: Some(item.guid.clone()),
                                raw_link: Some(item.link.clone()),
                                title: Some(item.title.clone()),
                                telegraph_url: telegraph_url.clone(),
                                created_at: None,
                                updated_at: None,
                            })
                            .await?;

                        self.broadcast_item(&source, item, &hash_id, telegraph_url.as_deref(), &subs).await?;
                    }

                    let next = next_fetch_at(now, self.config.update_interval);
                    if !self.options.dry_run {
                        self.repo
                            .mark_source_success(source.id, feed.etag.as_deref(), feed.last_modified.as_deref(), next)
                            .await?;
                        self.repo.prune_contents(source.id, self.config.fetch.retention_days, 200).await?;
                    }
                }
                Err(err) => {
                    warn!(source_id = source.id, error = %err, "fetch source failed");
                    if !self.options.dry_run {
                        self.repo.mark_source_error(source.id, backoff_fetch_at(now, source.error_count.unwrap_or(0))).await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Port of Go's `Bot.BroadcastNews` inner loop for a single new item:
    /// render the configured template per-subscriber and send, honoring the
    /// per-subscriber notification/telegraph flags and tag. On `Forbidden`
    /// (bot blocked/kicked), auto-unsubscribe that user from the source.
    async fn broadcast_item(
        &self,
        source: &Source,
        item: &ParsedItem,
        hash_id: &str,
        telegraph_url: Option<&str>,
        subs: &[Subscribe],
    ) -> anyhow::Result<()> {
        for sub in subs {
            let Some(user_id) = sub.user_id else { continue };

            let preview_text = trim_description(item.description.as_deref().unwrap_or(""), self.config.preview_text);
            let enable_telegraph = sub.enable_telegraph == Some(1) && telegraph_url.is_some();
            let data = MessageData {
                source_title: source.title.as_deref().unwrap_or(""),
                content_title: &item.title,
                raw_link: &item.link,
                preview_text: &preview_text,
                telegraph_url: telegraph_url.unwrap_or(""),
                tags: sub.tag.as_deref().unwrap_or(""),
                enable_telegraph,
            };
            let text = match self.config.message_mode {
                MessageMode::Html => render_html(&data),
                MessageMode::Markdown => render_markdown(&data),
            };
            let options = SendOptions {
                disable_web_page_preview: self.config.disable_web_page_preview,
                disable_notification: sub.enable_notification != Some(1),
                parse_mode: self.config.message_mode,
            };

            match self.sender.send_text(user_id, &text, options).await {
                Ok(SendOutcome::Sent) => {}
                Ok(SendOutcome::Forbidden) => {
                    self.repo.unsubscribe_user(user_id, source.id).await?;
                }
                Err(err) => {
                    warn!(
                        source_id = source.id,
                        user_id,
                        hash_id,
                        error = %err,
                        "broadcast news error"
                    );
                }
            }
        }
        Ok(())
    }
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

fn next_fetch_at(now: i64, interval_minutes: u64) -> i64 {
    now + interval_minutes.max(1) as i64 * 60
}

fn backoff_fetch_at(now: i64, current_error_count: i64) -> i64 {
    let exponent = current_error_count.clamp(0, 6) as u32;
    let minutes = 2_i64.pow(exponent).min(360);
    now + minutes * 60
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        bot::sender::test_support::RecordingSender,
        db,
    };

    #[test]
    fn next_fetch_at_uses_at_least_one_minute() {
        assert_eq!(next_fetch_at(100, 0), 160);
        assert_eq!(next_fetch_at(100, 10), 700);
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        assert_eq!(backoff_fetch_at(0, 0), 60);
        assert_eq!(backoff_fetch_at(0, 3), 8 * 60);
        assert_eq!(backoff_fetch_at(0, 99), 64 * 60);
    }

    #[tokio::test]
    async fn broadcast_item_sends_per_subscriber_and_auto_unsubscribes_forbidden() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::connect(dir.path().join("data.db").to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool);

        let source_id = repo.insert_source("https://example.com/feed", "Example Source").await.unwrap();
        repo.subscribe_user(1, source_id).await.unwrap();
        repo.subscribe_user(2, source_id).await.unwrap();
        repo.set_subscription_tag(1, source_id, "#tag").await.unwrap();

        let source = repo.source_by_link("https://example.com/feed").await.unwrap().unwrap();
        let subs = repo.subscribes_for_source(source_id).await.unwrap();
        assert_eq!(subs.len(), 2);

        let config = Config::default();
        let fetcher = Fetcher::new(&config).unwrap();
        let sender = RecordingSender { forbidden_chat_ids: vec![2], ..Default::default() };
        let scheduler = Scheduler::new(
            repo.clone(),
            fetcher,
            crate::preview::NoopPublisher,
            sender,
            config,
            SchedulerOptions::default(),
        );

        let item = ParsedItem {
            guid: "post-1".to_owned(),
            link: "https://example.com/post-1".to_owned(),
            title: "New Post".to_owned(),
            description: Some("<p>hello</p>".to_owned()),
            content: None,
        };

        scheduler.broadcast_item(&source, &item, "hash1", None, &subs).await.unwrap();

        {
            let sent = scheduler.sender.sent.lock().unwrap();
            assert_eq!(sent.len(), 2);
            assert!(sent.iter().any(|s| s.chat_id == 1 && s.text.contains("New Post")));
            assert!(sent.iter().any(|s| s.chat_id == 2));
        }

        assert!(repo.subscription(1, source_id).await.unwrap().is_some());
        assert!(repo.subscription(2, source_id).await.unwrap().is_none());
    }
}
