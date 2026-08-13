use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tracing::{info, warn};

use crate::{
    bot::{
        broadcast::{send_item_to_chat, ItemForChat, SubOptions},
        sender::{MessageSender, SendOutcome},
    },
    config::Config,
    db::{
        models::{Content, Source, Subscribe},
        repo::Repo,
    },
    feed::{
        fetch::{FetchOutcome, Fetcher},
        hash::gen_hash_id,
        parse::{is_stale_item, parse_feed, ParsedItem},
    },
    preview::{PublishRequest, PreviewPublisher},
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
                    // A single malformed feed used to `?` out of the whole
                    // batch, starving every later due source and never even
                    // recording the failure against this one.
                    let parsed = match parse_feed(&feed.body) {
                        Ok(parsed) => parsed,
                        Err(err) => {
                            warn!(source_id = source.id, error = %err, "parse source failed");
                            if !self.options.dry_run {
                                self.repo
                                    .mark_source_error(
                                        source.id,
                                        backoff_fetch_at(now, source.error_count.unwrap_or(0)),
                                        &format!("parse failed: {err}"),
                                    )
                                    .await?;
                            }
                            continue;
                        }
                    };
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
                        // Record it as seen before skipping, so a stale item is
                        // judged once and never re-evaluated — and so a ledger
                        // hole (GUID churn, `prune_contents`) heals instead of
                        // re-announcing the archive on every pass.
                        if is_stale_item(item.published, now, self.config.fetch.max_item_age_days) {
                            info!(
                                source_id = source.id,
                                hash_id = %hash_id,
                                title = %item.title,
                                published = ?item.published,
                                "skipping stale item"
                            );
                            if !self.options.dry_run {
                                self.repo.insert_content(&ledger_entry(source.id, item, &hash_id, None)).await?;
                            }
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
                                base_url: Some(&item.link),
                            })
                            .await
                            .unwrap_or_else(|err| {
                                warn!(source_id = source.id, %hash_id, error = %err, "telegraph publish failed");
                                None
                            });

                        self.repo
                            .insert_content(&ledger_entry(source.id, item, &hash_id, telegraph_url.clone()))
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
                        self.repo
                            .mark_source_error(
                                source.id,
                                backoff_fetch_at(now, source.error_count.unwrap_or(0)),
                                &err.to_string(),
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Port of Go's `Bot.BroadcastNews` inner loop for a single new item:
    /// render the configured template per-subscriber and send, honoring the
    /// per-subscriber notification/telegraph flags and tag. On `Forbidden`,
    /// keep the subscription and only log the failure: deleting subscriptions
    /// from the send path proved too risky because one bad send during a manual
    /// check/import flow could make `/list` appear empty.
    async fn broadcast_item(
        &self,
        source: &Source,
        item: &ParsedItem,
        hash_id: &str,
        telegraph_url: Option<&str>,
        subs: &[Subscribe],
    ) -> anyhow::Result<()> {
        // One opt-out read per item (opt-out rows only, so the set is small;
        // absence means the 🔖 button is enabled).
        let bm_off = self
            .repo
            .chat_ids_with_option_off(crate::bot::bookmarks::BM_BTN_PREFIX)
            .await
            .unwrap_or_default();
        // The 📝 summary button only exists when an MCP bridge is configured.
        let summary_configured = self.config.bookmark.ai.mcp.is_configured();
        let sum_off = if summary_configured {
            self.repo
                .chat_ids_with_option_off(crate::bot::bookmarks::BM_SUM_PREFIX)
                .await
                .unwrap_or_default()
        } else {
            std::collections::HashSet::new()
        };

        let item_data = ItemForChat {
            source_title: source.title.as_deref().unwrap_or(""),
            content_title: &item.title,
            raw_link: &item.link,
            description: item.description.as_deref().unwrap_or(""),
            telegraph_url,
            hash_id,
        };

        for sub in subs {
            let Some(user_id) = sub.user_id else { continue };

            let sub_opts = SubOptions {
                enable_notification: sub.enable_notification == Some(1),
                enable_telegraph: sub.enable_telegraph == Some(1),
                tag: sub.tag.as_deref().unwrap_or(""),
            };
            let bookmark_button = !bm_off.contains(&user_id);
            let summary_button = summary_configured && !sum_off.contains(&user_id);

            match send_item_to_chat(&self.sender, &self.config, user_id, &item_data, &sub_opts, bookmark_button, summary_button).await {
                Ok(SendOutcome::Sent) => {}
                Ok(SendOutcome::Forbidden) => {
                    warn!(source_id = source.id, user_id, hash_id, "broadcast forbidden; subscription kept");
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

/// Builds the `contents` dedup-ledger row for a feed item. Shared with the
/// manual `/check` pipeline, which duplicates this loop inline — both paths
/// must write the ledger identically or one would re-announce what the other
/// already sent. `telegraph_url` is `None` for items recorded without sending
/// (the stale-item gate), which is also what a failed publish stores.
pub(crate) fn ledger_entry(
    source_id: i64,
    item: &ParsedItem,
    hash_id: &str,
    telegraph_url: Option<String>,
) -> Content {
    Content {
        source_id: Some(source_id),
        hash_id: hash_id.to_owned(),
        raw_id: Some(item.guid.clone()),
        raw_link: Some(item.link.clone()),
        title: Some(item.title.clone()),
        telegraph_url,
        created_at: None,
        updated_at: None,
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
        testutil::spawn_single_response_server,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Records `tracing` events whose `message` field equals "would send",
    /// standing in for the log-scraping step of the manual `--dry-run`
    /// verification procedure in `docs/02-bot-rewrite.md` §7 (this sandbox
    /// has no real production `data.db` to point the binary at).
    struct WouldSendCounter(std::sync::Arc<AtomicUsize>);

    struct MessageVisitor(Option<String>);
    impl tracing::field::Visit for MessageVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = Some(format!("{value:?}"));
            }
        }
    }

    impl tracing::Subscriber for WouldSendCounter {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut visitor = MessageVisitor(None);
            event.record(&mut visitor);
            if visitor.0.as_deref() == Some("would send") {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    #[tokio::test]
    async fn dry_run_against_preexisting_content_reannounces_nothing() {
        const FEED_BODY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>Existing Feed</title>
<item><guid>post-42</guid><title>Existing Post</title><link>https://example.com/post-42</link><description>d</description></item>
</channel></rss>"#;

        let source_link = spawn_single_response_server(FEED_BODY).await;

        let dir = tempfile::tempdir().unwrap();
        let pool = db::connect(dir.path().join("data.db").to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool);
        let source_id = repo.insert_source(&source_link, "Existing Feed").await.unwrap();

        // Simulate a production `data.db` that already ingested this article
        // on a prior run, using the exact same hash function the scheduler
        // uses (the whole point of the gate: if this were wrong, the item
        // below would look "new" and get re-announced).
        let hash_id = gen_hash_id(&source_link, "post-42");
        repo.insert_content(&Content {
            source_id: Some(source_id),
            hash_id: hash_id.clone(),
            raw_id: Some("post-42".to_owned()),
            raw_link: Some("https://example.com/post-42".to_owned()),
            title: Some("Existing Post".to_owned()),
            telegraph_url: None,
            created_at: None,
            updated_at: None,
        })
        .await
        .unwrap();

        let config = Config::default();
        let fetcher = Fetcher::new(&config).unwrap();
        let scheduler = Scheduler::new(
            repo.clone(),
            fetcher,
            crate::preview::NoopPublisher,
            crate::bot::sender::NoopSender,
            config,
            SchedulerOptions { dry_run: true, ..SchedulerOptions::default() },
        );

        let would_send_count = std::sync::Arc::new(AtomicUsize::new(0));
        let subscriber = WouldSendCounter(would_send_count.clone());
        // `#[tokio::test]` defaults to the current-thread runtime, so this
        // thread-local guard stays valid across the `.await` below.
        let _guard = tracing::subscriber::set_default(subscriber);
        scheduler.run_once().await.unwrap();
        drop(_guard);

        assert_eq!(would_send_count.load(Ordering::SeqCst), 0, "pre-existing article must not be re-announced");
    }

    /// Companion to the gate above: with no pre-existing content row for the
    /// same feed, the identical item must be flagged. Without this, a broken
    /// hash/dedup check could silently make every article look "already
    /// seen" and the zero-count assertion above would pass for the wrong
    /// reason.
    #[tokio::test]
    async fn dry_run_flags_genuinely_new_items() {
        const FEED_BODY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>New Feed</title>
<item><guid>post-1</guid><title>Brand New Post</title><link>https://example.com/post-1</link><description>d</description></item>
</channel></rss>"#;

        let source_link = spawn_single_response_server(FEED_BODY).await;

        let dir = tempfile::tempdir().unwrap();
        let pool = db::connect(dir.path().join("data.db").to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool);
        repo.insert_source(&source_link, "New Feed").await.unwrap();

        let config = Config::default();
        let fetcher = Fetcher::new(&config).unwrap();
        let scheduler = Scheduler::new(
            repo,
            fetcher,
            crate::preview::NoopPublisher,
            crate::bot::sender::NoopSender,
            config,
            SchedulerOptions { dry_run: true, ..SchedulerOptions::default() },
        );

        let would_send_count = std::sync::Arc::new(AtomicUsize::new(0));
        let subscriber = WouldSendCounter(would_send_count.clone());
        let _guard = tracing::subscriber::set_default(subscriber);
        scheduler.run_once().await.unwrap();
        drop(_guard);

        assert_eq!(would_send_count.load(Ordering::SeqCst), 1, "genuinely new article must be flagged");
    }

    /// The age gate: an item the ledger has never seen but whose `<pubDate>`
    /// predates the cutoff is recorded as seen and *not* sent. This is what
    /// stops a feed that churned its GUIDs — or whose ledger rows aged out via
    /// `prune_contents` — from republishing its whole archive. An item with no
    /// date at all is unjudgeable and must still go out.
    #[tokio::test]
    async fn stale_items_are_recorded_but_never_sent() {
        // Models the reported symptom: a ledger with no rows for this source
        // (GUID churn, or `prune_contents` aged them out) plus a feed that
        // serves its whole back catalogue.
        const FEED_BODY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>Archive Feed</title>
<item><guid>old-1</guid><title>Post from 2013</title><link>https://example.com/old-1</link><description>d</description><pubDate>Fri, 01 Mar 2013 00:00:00 GMT</pubDate></item>
<item><guid>old-2</guid><title>Post from 2016</title><link>https://example.com/old-2</link><description>d</description><pubDate>Tue, 01 Mar 2016 00:00:00 GMT</pubDate></item>
<item><guid>old-3</guid><title>Post from 2020</title><link>https://example.com/old-3</link><description>d</description><pubDate>Sun, 01 Mar 2020 00:00:00 GMT</pubDate></item>
<item><guid>undated-1</guid><title>Undated Post</title><link>https://example.com/undated-1</link><description>d</description></item>
</channel></rss>"#;

        let source_link = spawn_single_response_server(FEED_BODY).await;

        let dir = tempfile::tempdir().unwrap();
        let pool = db::connect(dir.path().join("data.db").to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool);
        let source_id = repo.insert_source(&source_link, "Archive Feed").await.unwrap();
        repo.subscribe_user(1, source_id).await.unwrap();

        let config = Config::default();
        assert_eq!(config.fetch.max_item_age_days, 30, "test relies on the gate being on by default");
        let fetcher = Fetcher::new(&config).unwrap();
        let scheduler = Scheduler::new(
            repo.clone(),
            fetcher,
            crate::preview::NoopPublisher,
            RecordingSender::default(),
            config,
            SchedulerOptions::default(),
        );

        scheduler.run_once().await.unwrap();

        {
            let sent = scheduler.sender.sent.lock().unwrap();
            assert_eq!(sent.len(), 1, "the back catalogue must not be pushed");
            assert!(sent[0].text.contains("Undated Post"));
        }

        // Every item is in the ledger, stale ones included: they are silenced
        // permanently rather than re-judged (and re-skipped) on every pass —
        // which is also what heals the ledger hole that caused this.
        let hashes: Vec<String> = ["old-1", "old-2", "old-3", "undated-1"]
            .iter()
            .map(|guid| gen_hash_id(&source_link, guid))
            .collect();
        let existing = repo.existing_hash_ids(source_id, &hashes).await.unwrap();
        assert_eq!(existing.len(), 4, "stale items must still be recorded as seen");
    }

    /// A single malformed feed used to `?` out of `run_once`, starving every
    /// later due source for that pass and never recording the failure.
    #[tokio::test]
    async fn parse_failure_is_recorded_as_a_source_error() {
        let source_link = spawn_single_response_server("<html>not a feed at all</html>").await;

        let dir = tempfile::tempdir().unwrap();
        let pool = db::connect(dir.path().join("data.db").to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool);
        let source_id = repo.insert_source(&source_link, "Broken Feed").await.unwrap();

        let config = Config::default();
        let fetcher = Fetcher::new(&config).unwrap();
        let scheduler = Scheduler::new(
            repo.clone(),
            fetcher,
            crate::preview::NoopPublisher,
            crate::bot::sender::NoopSender,
            config,
            SchedulerOptions::default(),
        );

        scheduler.run_once().await.unwrap();

        let source = repo.get_source(source_id).await.unwrap().unwrap();
        assert_eq!(source.error_count, Some(1), "parse failure must count against the source");
        assert!(source.next_fetch_at > 0, "parse failure must schedule a backoff retry");
    }

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
    async fn broadcast_item_sends_per_subscriber_and_keeps_forbidden_subscription() {
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
            published: None,
        };

        scheduler.broadcast_item(&source, &item, "hash1", None, &subs).await.unwrap();

        {
            let sent = scheduler.sender.sent.lock().unwrap();
            assert_eq!(sent.len(), 2);
            assert!(sent.iter().any(|s| s.chat_id == 1 && s.text.contains("New Post")));
            assert!(sent.iter().any(|s| s.chat_id == 2));
        }

        assert!(repo.subscription(1, source_id).await.unwrap().is_some());
        assert!(repo.subscription(2, source_id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn broadcast_item_attaches_bookmark_button_unless_chat_opted_out() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::connect(dir.path().join("data.db").to_str().unwrap()).await.unwrap();
        let repo = Repo::new(pool);

        let source_id = repo.insert_source("https://example.com/feed", "Example Source").await.unwrap();
        repo.subscribe_user(1, source_id).await.unwrap();
        repo.subscribe_user(2, source_id).await.unwrap();
        // Chat 2 opted out of the 🔖 button.
        repo.set_option("tg-kl-vault:bmbtn:2", "0").await.unwrap();

        let source = repo.source_by_link("https://example.com/feed").await.unwrap().unwrap();
        let subs = repo.subscribes_for_source(source_id).await.unwrap();

        let config = Config::default();
        let fetcher = Fetcher::new(&config).unwrap();
        let scheduler = Scheduler::new(
            repo.clone(),
            fetcher,
            crate::preview::NoopPublisher,
            RecordingSender::default(),
            config,
            SchedulerOptions::default(),
        );

        let item = ParsedItem {
            guid: "post-1".to_owned(),
            link: "https://example.com/post-1".to_owned(),
            title: "New Post".to_owned(),
            description: Some("<p>hello</p>".to_owned()),
            content: None,
            published: None,
        };
        scheduler.broadcast_item(&source, &item, "hash1", None, &subs).await.unwrap();

        let sent = scheduler.sender.sent.lock().unwrap();
        let chat1 = sent.iter().find(|s| s.chat_id == 1).unwrap();
        let chat2 = sent.iter().find(|s| s.chat_id == 2).unwrap();
        assert!(chat1.reply_markup.is_some(), "opted-in chat gets the 🔖 button");
        assert!(chat2.reply_markup.is_none(), "opted-out chat gets no button");
    }
}
