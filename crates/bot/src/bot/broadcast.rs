//! Shared per-subscriber send path. Both the scheduler's `broadcast_item` and
//! the manual `/check` pipeline call `send_item_to_chat`, so the 🔖 button (and
//! anything else on the send path) can't end up wired to only one of them.

use crate::bot::bookmarks::add_button_markup;
use crate::bot::render::{render_html, render_markdown, MessageData};
use crate::bot::sender::{MessageSender, SendOptions, SendOutcome};
use crate::config::{Config, MessageMode};
use crate::preview::trim_description;

/// Per-content fields for one broadcast.
pub struct ItemForChat<'a> {
    pub source_title: &'a str,
    pub content_title: &'a str,
    pub raw_link: &'a str,
    pub description: &'a str,
    pub telegraph_url: Option<&'a str>,
    /// Content hash, encoded into the 🔖 button's `bm:add:<hash>` callback.
    pub hash_id: &'a str,
}

/// Per-subscriber display flags (normalized from `Subscribe` /
/// `SubscriptionSource`, which differ in shape).
pub struct SubOptions<'a> {
    pub enable_notification: bool,
    pub enable_telegraph: bool,
    pub tag: &'a str,
}

/// Renders and sends one item to one chat, optionally attaching the 🔖 button.
pub async fn send_item_to_chat<S: MessageSender>(
    sender: &S,
    config: &Config,
    chat_id: i64,
    item: &ItemForChat<'_>,
    sub: &SubOptions<'_>,
    bookmark_button: bool,
) -> anyhow::Result<SendOutcome> {
    let preview_text = trim_description(item.description, config.preview_text);
    let enable_telegraph = sub.enable_telegraph && item.telegraph_url.is_some();
    let data = MessageData {
        source_title: item.source_title,
        content_title: item.content_title,
        raw_link: item.raw_link,
        preview_text: &preview_text,
        telegraph_url: item.telegraph_url.unwrap_or(""),
        tags: sub.tag,
        enable_telegraph,
    };
    let text = match config.message_mode {
        MessageMode::Html => render_html(&data),
        MessageMode::Markdown => render_markdown(&data),
    };
    let options = SendOptions {
        disable_web_page_preview: config.disable_web_page_preview,
        disable_notification: !sub.enable_notification,
        parse_mode: config.message_mode,
    };
    let markup = bookmark_button.then(|| add_button_markup(item.hash_id));
    sender.send_text(chat_id, &text, options, markup).await
}
