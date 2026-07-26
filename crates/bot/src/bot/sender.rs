use teloxide::{prelude::*, types::LinkPreviewOptions};
use tracing::warn;

use crate::{config::MessageMode, ratelimit::SendRateLimiter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendOptions {
    pub disable_web_page_preview: bool,
    pub disable_notification: bool,
    pub parse_mode: MessageMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    /// The bot has been blocked/kicked by the recipient. Matches the Go
    /// original's substring check on `"Forbidden"` in the API error text;
    /// callers should auto-unsubscribe on this outcome (see `BroadcastNews`).
    Forbidden,
}

/// Abstraction over sending a Telegram text message, so the scheduler can be
/// exercised in tests without a live bot token or network access.
#[allow(async_fn_in_trait)]
pub trait MessageSender: Send + Sync {
    async fn send_text(&self, chat_id: i64, text: &str, options: SendOptions) -> anyhow::Result<SendOutcome>;
}

/// Sender used for `--dry-run`: the scheduler never actually calls it (dry
/// runs skip the send path entirely), but a concrete type is needed to
/// instantiate `Scheduler<P, S>` without a bot token.
#[derive(Debug, Clone, Default)]
pub struct NoopSender;

impl MessageSender for NoopSender {
    async fn send_text(&self, _chat_id: i64, _text: &str, _options: SendOptions) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Sent)
    }
}

/// Real sender backed by `teloxide::Bot`, rate limited to stay under
/// Telegram's global/per-chat send limits (sanctioned deviation D6).
pub struct TeloxideSender {
    bot: Bot,
    limiter: SendRateLimiter,
}

impl TeloxideSender {
    pub fn new(bot: Bot) -> Self {
        Self { bot, limiter: SendRateLimiter::default() }
    }
}

impl MessageSender for TeloxideSender {
    async fn send_text(&self, chat_id: i64, text: &str, options: SendOptions) -> anyhow::Result<SendOutcome> {
        self.limiter.until_ready(chat_id).await;

        let parse_mode = match options.parse_mode {
            MessageMode::Html => teloxide::types::ParseMode::Html,
            MessageMode::Markdown => teloxide::types::ParseMode::MarkdownV2,
        };
        let mut request = self
            .bot
            .send_message(ChatId(chat_id), text)
            .parse_mode(parse_mode)
            .disable_notification(options.disable_notification);
        if options.disable_web_page_preview {
            request = request.link_preview_options(LinkPreviewOptions {
                is_disabled: true,
                url: None,
                prefer_small_media: false,
                prefer_large_media: false,
                show_above_text: false,
            });
        }

        match request.await {
            Ok(_) => Ok(SendOutcome::Sent),
            Err(err) => {
                let message = err.to_string();
                if message.contains("Forbidden") {
                    warn!(chat_id, error = %message, "broadcast news error, bot stopped by user");
                    return Ok(SendOutcome::Forbidden);
                }
                // Telegram returns this when a markdown/HTML message has
                // incomplete formatting. Log the offending body, same as Go.
                if message.contains("parse entities") {
                    warn!(chat_id, markdown_msg = %text, error = %message, "broadcast news error, markdown error");
                }
                Err(anyhow::anyhow!(message))
            }
        }
    }
}

#[cfg(test)]
pub mod test_support {
    use std::sync::Mutex;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RecordedSend {
        pub chat_id: i64,
        pub text: String,
    }

    /// In-memory sender for scheduler tests: records sends, optionally
    /// reporting `SendOutcome::Forbidden` for specific chat ids.
    #[derive(Default)]
    pub struct RecordingSender {
        pub sent: Mutex<Vec<RecordedSend>>,
        pub forbidden_chat_ids: Vec<i64>,
    }

    impl MessageSender for RecordingSender {
        async fn send_text(&self, chat_id: i64, text: &str, _options: SendOptions) -> anyhow::Result<SendOutcome> {
            self.sent.lock().unwrap().push(RecordedSend { chat_id, text: text.to_owned() });
            if self.forbidden_chat_ids.contains(&chat_id) {
                return Ok(SendOutcome::Forbidden);
            }
            Ok(SendOutcome::Sent)
        }
    }
}
