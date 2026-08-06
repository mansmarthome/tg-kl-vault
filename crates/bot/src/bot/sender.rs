use teloxide::{
    prelude::*,
    types::{InlineKeyboardMarkup, LinkPreviewOptions, MessageId},
    ApiError, RequestError,
};
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
    /// callers should treat this as a send failure without deleting data.
    Forbidden,
}

/// Abstraction over sending a Telegram text message, so the scheduler can be
/// exercised in tests without a live bot token or network access.
///
/// `reply_markup` is a positional argument (not a `SendOptions` field): that
/// struct is `Copy` and `InlineKeyboardMarkup` is not. It is also not a second
/// `send_text_with_markup` method — a defaulted variant would let some caller
/// silently drop the 🔖 button, which is precisely the bug class this codebase
/// already has.
#[allow(async_fn_in_trait)]
pub trait MessageSender: Send + Sync {
    async fn send_text(
        &self,
        chat_id: i64,
        text: &str,
        options: SendOptions,
        reply_markup: Option<InlineKeyboardMarkup>,
    ) -> anyhow::Result<SendOutcome>;
}

/// Options for editing an existing message. `reply_markup` must live here:
/// `editMessageText` *clears* the keyboard if it isn't re-sent.
#[derive(Clone)]
pub struct EditOptions {
    pub parse_mode: MessageMode,
    pub disable_web_page_preview: bool,
    pub reply_markup: Option<InlineKeyboardMarkup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOutcome {
    Edited,
    /// Telegram returned "message is not modified" — a benign no-op (double
    /// tap / redelivered update), treated as success.
    NotModified,
    /// The message is gone or uneditable (deleted, too old, bot blocked). The
    /// worker clears `notify_message_id` and moves on; never a retry.
    Gone,
}

/// Editing an existing message. Kept separate from `MessageSender` so the tag
/// worker can be driven by a `RecordingEditor` in tests.
#[allow(async_fn_in_trait)]
pub trait MessageEditor: Send + Sync {
    async fn edit_text(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        options: EditOptions,
    ) -> anyhow::Result<EditOutcome>;

    async fn edit_markup(
        &self,
        chat_id: i64,
        message_id: i32,
        markup: InlineKeyboardMarkup,
    ) -> anyhow::Result<EditOutcome>;
}

/// Sender used for `--dry-run`: the scheduler never actually calls it (dry
/// runs skip the send path entirely), but a concrete type is needed to
/// instantiate `Scheduler<P, S>` without a bot token.
#[derive(Debug, Clone, Default)]
pub struct NoopSender;

impl MessageSender for NoopSender {
    async fn send_text(
        &self,
        _chat_id: i64,
        _text: &str,
        _options: SendOptions,
        _reply_markup: Option<InlineKeyboardMarkup>,
    ) -> anyhow::Result<SendOutcome> {
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

fn disabled_preview() -> LinkPreviewOptions {
    LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    }
}

/// Maps a teloxide error on an edit request to an `EditOutcome`, using typed
/// `ApiError` matching (not the frozen Go-parity substring check). Returns
/// `Err` for anything genuinely unexpected. `ApiError` is `#[non_exhaustive]`.
fn classify_edit_error(err: &RequestError) -> Option<EditOutcome> {
    match err {
        RequestError::Api(ApiError::MessageToEditNotFound)
        | RequestError::Api(ApiError::MessageCantBeEdited)
        | RequestError::Api(ApiError::BotBlocked) => Some(EditOutcome::Gone),
        RequestError::Api(ApiError::MessageNotModified) => Some(EditOutcome::NotModified),
        _ => None,
    }
}

impl MessageSender for TeloxideSender {
    async fn send_text(
        &self,
        chat_id: i64,
        text: &str,
        options: SendOptions,
        reply_markup: Option<InlineKeyboardMarkup>,
    ) -> anyhow::Result<SendOutcome> {
        self.limiter.until_ready(chat_id).await;

        let parse_mode = match options.parse_mode {
            MessageMode::Html => teloxide::types::ParseMode::Html,
            MessageMode::Markdown => teloxide::types::ParseMode::MarkdownV2,
        };
        let send = || {
            let mut request = self
                .bot
                .send_message(ChatId(chat_id), text)
                .parse_mode(parse_mode)
                .disable_notification(options.disable_notification);
            if options.disable_web_page_preview {
                request = request.link_preview_options(disabled_preview());
            }
            // The closure runs a second time on a 429 retry, so clone.
            if let Some(markup) = reply_markup.clone() {
                request = request.reply_markup(markup);
            }
            request
        };

        // Sanctioned deviation D6: on a 429, sleep for Telegram's requested
        // `retry_after` and retry exactly once, then give up and log.
        let result = match send().await {
            Err(teloxide::RequestError::RetryAfter(seconds)) => {
                warn!(chat_id, retry_after_secs = seconds.seconds(), "send hit 429, retrying once");
                tokio::time::sleep(seconds.duration()).await;
                send().await
            }
            other => other,
        };

        match result {
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

impl MessageEditor for TeloxideSender {
    async fn edit_text(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        options: EditOptions,
    ) -> anyhow::Result<EditOutcome> {
        // Editing counts against the same Telegram limits as sending.
        self.limiter.until_ready(chat_id).await;

        let parse_mode = match options.parse_mode {
            MessageMode::Html => teloxide::types::ParseMode::Html,
            MessageMode::Markdown => teloxide::types::ParseMode::MarkdownV2,
        };
        let edit = || {
            let mut request = self
                .bot
                .edit_message_text(ChatId(chat_id), MessageId(message_id), text)
                .parse_mode(parse_mode);
            if options.disable_web_page_preview {
                request = request.link_preview_options(disabled_preview());
            }
            if let Some(markup) = options.reply_markup.clone() {
                request = request.reply_markup(markup);
            }
            request
        };

        match edit().await {
            Ok(_) => Ok(EditOutcome::Edited),
            Err(RequestError::RetryAfter(seconds)) => {
                warn!(chat_id, retry_after_secs = seconds.seconds(), "edit hit 429, retrying once");
                tokio::time::sleep(seconds.duration()).await;
                match edit().await {
                    Ok(_) => Ok(EditOutcome::Edited),
                    Err(err) => classify_edit_error(&err)
                        .ok_or_else(|| anyhow::anyhow!(err.to_string())),
                }
            }
            Err(err) => {
                classify_edit_error(&err).ok_or_else(|| anyhow::anyhow!(err.to_string()))
            }
        }
    }

    async fn edit_markup(
        &self,
        chat_id: i64,
        message_id: i32,
        markup: InlineKeyboardMarkup,
    ) -> anyhow::Result<EditOutcome> {
        self.limiter.until_ready(chat_id).await;
        let result = self
            .bot
            .edit_message_reply_markup(ChatId(chat_id), MessageId(message_id))
            .reply_markup(markup)
            .await;
        match result {
            Ok(_) => Ok(EditOutcome::Edited),
            Err(err) => {
                classify_edit_error(&err).ok_or_else(|| anyhow::anyhow!(err.to_string()))
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
        pub reply_markup: Option<InlineKeyboardMarkup>,
    }

    /// In-memory sender for scheduler tests: records sends, optionally
    /// reporting `SendOutcome::Forbidden` for specific chat ids.
    #[derive(Default)]
    pub struct RecordingSender {
        pub sent: Mutex<Vec<RecordedSend>>,
        pub forbidden_chat_ids: Vec<i64>,
    }

    impl MessageSender for RecordingSender {
        async fn send_text(
            &self,
            chat_id: i64,
            text: &str,
            _options: SendOptions,
            reply_markup: Option<InlineKeyboardMarkup>,
        ) -> anyhow::Result<SendOutcome> {
            self.sent.lock().unwrap().push(RecordedSend {
                chat_id,
                text: text.to_owned(),
                reply_markup,
            });
            if self.forbidden_chat_ids.contains(&chat_id) {
                return Ok(SendOutcome::Forbidden);
            }
            Ok(SendOutcome::Sent)
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RecordedEdit {
        pub chat_id: i64,
        pub message_id: i32,
        /// `Some(text)` for `edit_text`, `None` for a markup-only edit.
        pub text: Option<String>,
        pub reply_markup: Option<InlineKeyboardMarkup>,
    }

    /// In-memory editor for tag-worker tests. `gone_message_ids` makes a given
    /// message id report `EditOutcome::Gone`.
    #[derive(Default)]
    pub struct RecordingEditor {
        pub edits: Mutex<Vec<RecordedEdit>>,
        pub gone_message_ids: Vec<i32>,
    }

    impl MessageEditor for RecordingEditor {
        async fn edit_text(
            &self,
            chat_id: i64,
            message_id: i32,
            text: &str,
            options: EditOptions,
        ) -> anyhow::Result<EditOutcome> {
            self.edits.lock().unwrap().push(RecordedEdit {
                chat_id,
                message_id,
                text: Some(text.to_owned()),
                reply_markup: options.reply_markup,
            });
            if self.gone_message_ids.contains(&message_id) {
                return Ok(EditOutcome::Gone);
            }
            Ok(EditOutcome::Edited)
        }

        async fn edit_markup(
            &self,
            chat_id: i64,
            message_id: i32,
            markup: InlineKeyboardMarkup,
        ) -> anyhow::Result<EditOutcome> {
            self.edits.lock().unwrap().push(RecordedEdit {
                chat_id,
                message_id,
                text: None,
                reply_markup: Some(markup),
            });
            if self.gone_message_ids.contains(&message_id) {
                return Ok(EditOutcome::Gone);
            }
            Ok(EditOutcome::Edited)
        }
    }
}
