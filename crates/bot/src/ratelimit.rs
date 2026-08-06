use std::{collections::HashMap, time::{Duration, Instant}};

use tokio::sync::Mutex;

/// A single-slot minimum-interval gate: successive `until_ready` calls are
/// spaced at least `spacing` apart. Shared primitive behind both the Telegram
/// send limiter and the Gemini RPM interval.
#[derive(Debug)]
pub struct MinIntervalLimiter {
    next: Mutex<Instant>,
    spacing: Duration,
}

impl MinIntervalLimiter {
    pub fn new(spacing: Duration) -> Self {
        Self {
            next: Mutex::new(Instant::now()),
            spacing,
        }
    }

    pub async fn until_ready(&self) {
        let now = Instant::now();
        let sleep_for = {
            let mut next = self.next.lock().await;
            let sleep_for = next.saturating_duration_since(now);
            *next = now + sleep_for + self.spacing;
            sleep_for
        };
        if !sleep_for.is_zero() {
            tokio::time::sleep(sleep_for).await;
        }
    }
}

/// Conservative Telegram send limiter: one global token every ~40ms (~25/s)
/// and one per-chat token every 3s (~20/min). This is intentionally simple;
/// send code can still honor Telegram `retry_after` on 429 separately.
#[derive(Debug)]
pub struct SendRateLimiter {
    global: MinIntervalLimiter,
    per_chat_next: Mutex<HashMap<i64, Instant>>,
    per_chat_spacing: Duration,
}

impl Default for SendRateLimiter {
    fn default() -> Self {
        Self {
            global: MinIntervalLimiter::new(Duration::from_millis(40)),
            per_chat_next: Mutex::new(HashMap::new()),
            per_chat_spacing: Duration::from_secs(3),
        }
    }
}

impl SendRateLimiter {
    pub async fn until_ready(&self, chat_id: i64) {
        self.global.until_ready().await;

        let now = Instant::now();
        let sleep_for = {
            let mut by_chat = self.per_chat_next.lock().await;
            let next = by_chat.entry(chat_id).or_insert(now);
            let sleep_for = next.saturating_duration_since(now);
            *next = now + sleep_for + self.per_chat_spacing;
            sleep_for
        };
        if !sleep_for.is_zero() {
            tokio::time::sleep(sleep_for).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn limiter_first_send_is_immediate() {
        let limiter = SendRateLimiter::default();
        let start = Instant::now();
        limiter.until_ready(1).await;
        assert!(start.elapsed() < Duration::from_millis(100));
    }
}
