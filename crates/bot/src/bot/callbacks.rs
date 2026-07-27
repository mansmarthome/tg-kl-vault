use std::sync::Arc;

use teloxide::{
    prelude::*,
    types::{ChatId, MessageId, ParseMode},
};
use tracing::warn;

use crate::{
    bot::{
        callback::{decode_telebot_callback, Attachment, Button},
        keyboard::{feed_setting_keyboard, settings_keyboard},
        render::{render_feed_setting, FeedSettingData},
        runtime::{export_chat_opml, chat_lang, set_chat_lang, BotState, Lang},
    },
    config::ERROR_THRESHOLD,
    db::models::{Source, Subscribe},
};

/// Dispatches all 8 inline-button callbacks from `00-OVERVIEW.md` §3 /
/// `02-bot-rewrite.md` §4. Every button carries the same hex-encoded
/// `Attachment` payload (only the button `unique` differs), decoded once
/// here and passed to the per-button handler below.
pub async fn handle_callback(bot: Bot, query: CallbackQuery, state: Arc<BotState>) -> ResponseResult<()> {
    let Some(data) = query.data.as_deref() else { return Ok(()) };
    let Some(message) = query.regular_message() else { return Ok(()) };
    let chat_id = message.chat.id;
    let message_id = message.id;

    if let Some(action) = data.strip_prefix("settings:") {
        return handle_settings_callback(&bot, &query, &state, action, chat_id, message_id).await;
    }

    let callback = match decode_telebot_callback(data) {
        Ok(callback) => callback,
        Err(err) => {
            warn!(error = %err, "failed to decode callback data");
            respond_toast(&bot, &query, "系统错误！").await?;
            return Ok(());
        }
    };

    match callback.button {
        Button::SetFeedItem => {
            handle_set_feed_item(&bot, &query, &state, callback.attachment, chat_id, message_id).await
        }
        Button::SetToggleUpdate => {
            handle_toggle_update(&bot, &query, &state, callback.attachment, chat_id, message_id).await
        }
        Button::SetToggleNotice => {
            handle_toggle_notice(&bot, &query, &state, callback.attachment, chat_id, message_id).await
        }
        Button::SetToggleTelegraph => {
            handle_toggle_telegraph(&bot, &query, &state, callback.attachment, chat_id, message_id).await
        }
        Button::SetSetSubTag => handle_set_sub_tag(&bot, &query, callback.attachment, chat_id, message_id).await,
        Button::UnsubFeedItem => {
            handle_unsub_feed_item(&bot, &state, callback.attachment, chat_id, message_id).await
        }
        Button::UnsubAllConfirm => handle_unsuball_confirm(&bot, &state, &query, chat_id, message_id).await,
        Button::UnsubAllCancel => handle_unsuball_cancel(&bot, chat_id, message_id).await,
    }
}

/// Go's `SubscriptionSwitchButton` et al. check `attachData.GetUserId() !=
/// c.Sender.ID` and, on mismatch, verify the sender is a chat admin of the
/// subscriber chat/channel. That admin-delegation path needs live
/// `getChatAdministrators` calls we don't wire up in this pass, so a mismatch
/// is simply rejected here; group/channel co-management is a follow-up.
fn is_authorized(attachment: Attachment, query: &CallbackQuery) -> bool {
    attachment.user_id == query.from.id.0 as i64
}

async fn respond_toast(bot: &Bot, query: &CallbackQuery, text: &str) -> ResponseResult<()> {
    bot.answer_callback_query(query.id.clone()).text(text).await?;
    Ok(())
}

async fn edit_plain(bot: &Bot, chat_id: ChatId, message_id: MessageId, text: &str) -> ResponseResult<()> {
    bot.edit_message_text(chat_id, message_id, text).await?;
    Ok(())
}

async fn handle_settings_callback(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    action: &str,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let owner_id = chat_id.0;
    let current_lang = chat_lang(&state.repo, owner_id).await;
    match action {
        "import" => respond_toast(bot, query, current_lang.import_hint()).await,
        "export" => export_chat_opml(bot, chat_id, owner_id, state).await,
        "setinterval" => edit_plain(bot, chat_id, message_id, current_lang.interval_hint()).await,
        "lang:en" | "lang:zh-tw" => {
            let lang = if action.ends_with("en") { Lang::En } else { Lang::ZhTw };
            if let Err(err) = set_chat_lang(&state.repo, owner_id, lang).await {
                warn!(owner_id, error = %err, "failed to set language");
                return respond_toast(bot, query, "error").await;
            }
            respond_toast(bot, query, lang.lang_updated()).await?;
            bot.edit_message_text(chat_id, message_id, lang.settings_title())
                .reply_markup(settings_keyboard(lang))
                .await?;
            Ok(())
        }
        _ => respond_toast(bot, query, "error").await,
    }
}

async fn render_and_edit_setting(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    source: &Source,
    sub: &Subscribe,
    attachment: Attachment,
) -> ResponseResult<()> {
    let text = render_feed_setting(&FeedSettingData {
        source_id: source.id,
        source_title: source.title.as_deref().unwrap_or(""),
        source_link: source.link.as_deref().unwrap_or(""),
        source_error_count: source.error_count.unwrap_or(0),
        error_threshold: i64::from(ERROR_THRESHOLD),
        interval: sub.interval.unwrap_or(0),
        enable_notification: sub.enable_notification,
        enable_telegraph: sub.enable_telegraph,
        tag: sub.tag.as_deref().unwrap_or(""),
    });
    let keyboard = feed_setting_keyboard(
        attachment,
        source.error_count.unwrap_or(0),
        i64::from(ERROR_THRESHOLD),
        sub.enable_notification,
        sub.enable_telegraph,
    );
    bot.edit_message_text(chat_id, message_id, text).parse_mode(ParseMode::Html).reply_markup(keyboard).await?;
    Ok(())
}

async fn handle_set_feed_item(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    if !is_authorized(attachment, query) {
        return edit_plain(bot, chat_id, message_id, "获取订阅信息失败").await;
    }
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(source)) = state.repo.get_source(source_id).await else {
        return edit_plain(bot, chat_id, message_id, "找不到该订阅源").await;
    };
    let Ok(Some(sub)) = state.repo.subscription(attachment.user_id, source_id).await else {
        return edit_plain(bot, chat_id, message_id, "用户未订阅该rss").await;
    };
    render_and_edit_setting(bot, chat_id, message_id, &source, &sub, attachment).await
}

async fn handle_toggle_update(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    if !is_authorized(attachment, query) {
        return respond_toast(bot, query, "error").await;
    }
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(sub)) = state.repo.subscription(attachment.user_id, source_id).await else {
        return respond_toast(bot, query, "error").await;
    };
    let Ok(Some(source)) = state.repo.toggle_source_update_status(source_id).await else {
        return respond_toast(bot, query, "error").await;
    };
    respond_toast(bot, query, "修改成功").await?;
    render_and_edit_setting(bot, chat_id, message_id, &source, &sub, attachment).await
}

async fn handle_toggle_notice(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    if !is_authorized(attachment, query) {
        return edit_plain(bot, chat_id, message_id, "系统错误！").await;
    }
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(source)) = state.repo.get_source(source_id).await else {
        return respond_toast(bot, query, "error").await;
    };
    let Ok(Some(sub)) = state.repo.toggle_subscription_notice(attachment.user_id, source_id).await else {
        return respond_toast(bot, query, "error").await;
    };
    respond_toast(bot, query, "修改成功").await?;
    render_and_edit_setting(bot, chat_id, message_id, &source, &sub, attachment).await
}

async fn handle_toggle_telegraph(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    if !is_authorized(attachment, query) {
        return respond_toast(bot, query, "error").await;
    }
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(source)) = state.repo.get_source(source_id).await else {
        return respond_toast(bot, query, "error").await;
    };
    let Ok(Some(sub)) = state.repo.toggle_subscription_telegraph(attachment.user_id, source_id).await else {
        return respond_toast(bot, query, "error").await;
    };
    respond_toast(bot, query, "修改成功").await?;
    render_and_edit_setting(bot, chat_id, message_id, &source, &sub, attachment).await
}

// Go's `SetSubscriptionTagButton` replies with legacy `tb.ModeMarkdown`.
#[allow(deprecated)]
async fn handle_set_sub_tag(
    bot: &Bot,
    query: &CallbackQuery,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    if !is_authorized(attachment, query) {
        // Go's `feedSetAuth` failure sends a *new* message via `ctx.Send`,
        // unlike every other handler here which edits in place.
        bot.send_message(chat_id, "无权限").await?;
        return Ok(());
    }
    let source_id = attachment.source_id;
    let text = format!(
        "请使用`/setfeedtag {source_id} tags`命令为该订阅设置标签，tags为需要设置的标签，以空格分隔。（最多设置三个标签） \n例如：`/setfeedtag {source_id} 科技 苹果`"
    );
    bot.edit_message_text(chat_id, message_id, text).parse_mode(ParseMode::Markdown).await?;
    Ok(())
}

/// Mirrors `RemoveSubscriptionItemButton.Handle` in the Go source, which does
/// not compare `attachData.GetUserId()` against the callback sender at all.
/// Preserved verbatim rather than "fixed" per the ground rule to match
/// upstream behaviour exactly.
async fn handle_unsub_feed_item(
    bot: &Bot,
    state: &BotState,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(source)) = state.repo.get_source(source_id).await else {
        return edit_plain(bot, chat_id, message_id, "退订错误！").await;
    };
    match state.repo.unsubscribe_user(attachment.user_id, source_id).await {
        Ok(_) => {
            let text = format!(
                "[{source_id}] <a href=\"{}\">{}</a> 退订成功",
                source.link.as_deref().unwrap_or(""),
                source.title.as_deref().unwrap_or("")
            );
            bot.edit_message_text(chat_id, message_id, text).parse_mode(ParseMode::Html).await?;
            Ok(())
        }
        Err(_) => edit_plain(bot, chat_id, message_id, "退订错误！").await,
    }
}

async fn handle_unsuball_confirm(
    bot: &Bot,
    state: &BotState,
    query: &CallbackQuery,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let sender_id = query.from.id.0 as i64;
    match state.repo.unsubscribe_all_user(sender_id).await {
        Ok(_) => edit_plain(bot, chat_id, message_id, "退订成功").await,
        Err(_) => edit_plain(bot, chat_id, message_id, "退订失败").await,
    }
}

async fn handle_unsuball_cancel(bot: &Bot, chat_id: ChatId, message_id: MessageId) -> ResponseResult<()> {
    edit_plain(bot, chat_id, message_id, "操作取消").await
}
