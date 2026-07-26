use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tracing::{info, warn};

use crate::{
    config::Config,
    db::{models::Content, repo::Repo},
    feed::{fetch::{FetchOutcome, Fetcher}, hash::gen_hash_id, parse::parse_feed},
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

pub struct Scheduler<P> {
    repo: Repo,
    fetcher: Fetcher,
    publisher: P,
    config: Config,
    options: SchedulerOptions,
}

impl<P> Scheduler<P>
where
    P: PreviewPublisher,
{
    pub fn new(repo: Repo, fetcher: Fetcher, publisher: P, config: Config, options: SchedulerOptions) -> Self {
        Self { repo, fetcher, publisher, config, options }
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
                                hash_id,
                                raw_id: Some(item.guid.clone()),
                                raw_link: Some(item.link.clone()),
                                title: Some(item.title.clone()),
                                telegraph_url,
                                created_at: None,
                                updated_at: None,
                            })
                            .await?;
                    }

                    let next = next_fetch_at(now, self.config.update_interval);
                    if !self.options.dry_run {
                        self.repo
                            .mark_source_success(source.id, feed.etag.as_deref(), feed.last_modified.as_deref(), next)
                            .await?;
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
    use super::{backoff_fetch_at, next_fetch_at};

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
}
