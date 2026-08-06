//! Daily soft budget for Gemini calls. A **single** `options` row
//! `tg-kl-vault:ai:quota` holding `"YYYY-MM-DD:count"`, reset when the date
//! rolls. One row per day would grow unbounded (nothing prunes `options`).
//!
//! This is only a conservative guard; the 429 latch in the Gemini client is
//! the authoritative limit. Google resets at Pacific midnight — we use UTC and
//! deliberately do NOT pull in `chrono-tz` for a soft counter.

use chrono::Utc;

use crate::db::repo::Repo;

const QUOTA_KEY: &str = "tg-kl-vault:ai:quota";

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

fn parse(value: &str) -> (String, u32) {
    match value.split_once(':') {
        Some((date, count)) => (date.to_owned(), count.parse().unwrap_or(0)),
        None => (String::new(), 0),
    }
}

/// Attempts to consume one unit of today's budget. Returns `true` if allowed
/// (and records the consumption), `false` if today's soft quota is exhausted.
pub async fn try_consume(repo: &Repo, daily_quota: u32) -> anyhow::Result<bool> {
    let today = today();
    let (date, count) = repo
        .get_option(QUOTA_KEY)
        .await?
        .map(|v| parse(&v))
        .unwrap_or_default();
    let current = if date == today { count } else { 0 };
    if current >= daily_quota {
        return Ok(false);
    }
    repo.set_option(QUOTA_KEY, &format!("{today}:{}", current + 1))
        .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn repo() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::connect(dir.path().join("q.db").to_str().unwrap()).await.unwrap();
        std::mem::forget(dir);
        Repo::new(pool)
    }

    #[tokio::test]
    async fn consumes_until_quota_then_refuses() {
        let repo = repo().await;
        assert!(try_consume(&repo, 2).await.unwrap());
        assert!(try_consume(&repo, 2).await.unwrap());
        assert!(!try_consume(&repo, 2).await.unwrap());
        // The counter is a single row, not one-per-call.
        let stored = repo.get_option(QUOTA_KEY).await.unwrap().unwrap();
        assert!(stored.ends_with(":2"));
    }
}
