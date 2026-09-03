use std::sync::Arc;

use teloxide::{
    prelude::*,
    types::{ChatId, MessageId, ParseMode},
};
use tracing::warn;

use crate::{
    bot::{
        callback::{decode_telebot_callback, Attachment, Button},
        keyboard::{
            feed_setting_keyboard, settings_interval_keyboard, settings_keyboard,
            settings_language_keyboard, settings_opml_keyboard,
        },
        render::{render_feed_setting, FeedSettingData},
        runtime::{chat_lang, export_chat_opml, set_chat_lang, BotState, Lang},
    },
    config::ERROR_THRESHOLD,
    db::models::{Source, Subscribe},
};

/// Dispatches all 8 inline-button callbacks from `00-OVERVIEW.md` §3 /
/// `02-bot-rewrite.md` §4. Every button carries the same hex-encoded
/// `Attachment` payload (only the button `unique` differs), decoded once
/// here and passed to the per-button handler below.
pub async fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    state: Arc<BotState>,
) -> ResponseResult<()> {
    let Some(data) = query.data.as_deref() else {
        return Ok(());
    };
    let Some(message) = query.regular_message() else {
        return Ok(());
    };
    let chat_id = message.chat.id;
    let message_id = message.id;

    if let Some(action) = data.strip_prefix("settings:") {
        return handle_settings_callback(&bot, &query, &state, action, chat_id, message_id).await;
    }

    let callback = match decode_telebot_callback(data) {
        Ok(callback) => callback,
        Err(err) => {
            warn!(error = %err, "failed to decode callback data");
            let lang = chat_lang(&state.repo, chat_id.0).await;
            respond_toast(&bot, &query, lang.system_error_exclaim()).await?;
            return Ok(());
        }
    };

    match callback.button {
        Button::SetFeedItem => {
            handle_set_feed_item(
                &bot,
                &query,
                &state,
                callback.attachment,
                chat_id,
                message_id,
            )
            .await
        }
        Button::SetToggleUpdate => {
            handle_toggle_update(
                &bot,
                &query,
                &state,
                callback.attachment,
                chat_id,
                message_id,
            )
            .await
        }
        Button::SetToggleNotice => {
            handle_toggle_notice(
                &bot,
                &query,
                &state,
                callback.attachment,
                chat_id,
                message_id,
            )
            .await
        }
        Button::SetToggleTelegraph => {
            handle_toggle_telegraph(
                &bot,
                &query,
                &state,
                callback.attachment,
                chat_id,
                message_id,
            )
            .await
        }
        Button::SetToggleSourceTitle => {
            handle_toggle_source_title(
                &bot,
                &query,
                &state,
                callback.attachment,
                chat_id,
                message_id,
            )
            .await
        }
        Button::SetSetSubTag => {
            handle_set_sub_tag(&bot, &state, &query, callback.attachment, chat_id, message_id)
                .await
        }
        Button::UnsubFeedItem => {
            handle_unsub_feed_item(&bot, &state, callback.attachment, chat_id, message_id).await
        }
        Button::UnsubAllConfirm => {
            handle_unsuball_confirm(&bot, &state, &query, chat_id, message_id).await
        }
        Button::UnsubAllCancel => {
            let lang = chat_lang(&state.repo, chat_id.0).await;
            handle_unsuball_cancel(&bot, chat_id, message_id, lang).await
        }
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
    bot.answer_callback_query(query.id.clone())
        .text(text)
        .await?;
    Ok(())
}

async fn edit_plain(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    text: &str,
) -> ResponseResult<()> {
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
        "back" => {
            bot.edit_message_text(chat_id, message_id, current_lang.settings_title())
                .reply_markup(settings_keyboard(current_lang))
                .await?;
            Ok(())
        }
        "opml" => {
            bot.edit_message_text(chat_id, message_id, current_lang.settings_opml_button())
                .reply_markup(settings_opml_keyboard(current_lang))
                .await?;
            Ok(())
        }
        "opml:import" => {
            bot.edit_message_text(chat_id, message_id, current_lang.import_hint())
                .reply_markup(settings_opml_keyboard(current_lang))
                .await?;
            Ok(())
        }
        "opml:export" => export_chat_opml(bot, chat_id, owner_id, state, current_lang).await,
        "interval" => {
            bot.edit_message_text(chat_id, message_id, current_lang.interval_hint())
                .reply_markup(settings_interval_keyboard(current_lang))
                .await?;
            Ok(())
        }
        action if action.starts_with("interval:") => {
            let Some(minutes) = action
                .strip_prefix("interval:")
                .and_then(|v| v.parse::<i64>().ok())
            else {
                return respond_toast(bot, query, current_lang.toast_error()).await;
            };
            match state
                .repo
                .set_all_subscription_interval(owner_id, minutes)
                .await
            {
                Ok(count) => {
                    respond_toast(bot, query, &current_lang.interval_updated(count)).await
                }
                Err(err) => {
                    warn!(owner_id, minutes, error = %err, "failed to set interval");
                    respond_toast(bot, query, current_lang.toast_error()).await
                }
            }
        }
        "language" => {
            bot.edit_message_text(chat_id, message_id, current_lang.settings_language_button())
                .reply_markup(settings_language_keyboard(current_lang))
                .await?;
            Ok(())
        }
        "language:en" | "language:zh-tw" | "language:ru" => {
            let lang = if action.ends_with("en") {
                Lang::En
            } else if action.ends_with("zh-tw") {
                Lang::ZhTw
            } else {
                Lang::Ru
            };
            if let Err(err) = set_chat_lang(&state.repo, owner_id, lang).await {
                warn!(owner_id, error = %err, "failed to set language");
                return respond_toast(bot, query, current_lang.toast_error()).await;
            }
            // Use the *new* language for the confirmation toast.
            respond_toast(bot, query, lang.lang_updated(lang)).await?;
            bot.edit_message_text(chat_id, message_id, lang.settings_title())
                .reply_markup(settings_keyboard(lang))
                .await?;
            Ok(())
        }
        _ => respond_toast(bot, query, current_lang.toast_error()).await,
    }
}

async fn render_and_edit_setting(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    source: &Source,
    sub: &Subscribe,
    attachment: Attachment,
    lang: Lang,
) -> ResponseResult<()> {
    let data = FeedSettingData {
        source_id: source.id,
        source_title: source.title.as_deref().unwrap_or(""),
        source_link: source.link.as_deref().unwrap_or(""),
        source_error_count: source.error_count.unwrap_or(0),
        error_threshold: i64::from(ERROR_THRESHOLD),
        interval: sub.interval.unwrap_or(0),
        enable_notification: sub.enable_notification,
        enable_telegraph: sub.enable_telegraph,
        enable_source_title: sub.enable_source_title,
        tag: sub.tag.as_deref().unwrap_or(""),
    };
    let text = render_feed_setting(lang, &data);
    let keyboard = feed_setting_keyboard(
        attachment,
        source.error_count.unwrap_or(0),
        i64::from(ERROR_THRESHOLD),
        sub.enable_notification,
        sub.enable_telegraph,
        sub.enable_source_title,
        lang,
    );
    bot.edit_message_text(chat_id, message_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
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
    let lang = chat_lang(&state.repo, chat_id.0).await;
    if !is_authorized(attachment, query) {
        return edit_plain(bot, chat_id, message_id, lang.set_unauthorized()).await;
    }
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(source)) = state.repo.get_source(source_id).await else {
        return edit_plain(bot, chat_id, message_id, lang.set_source_not_found()).await;
    };
    let Ok(Some(sub)) = state.repo.subscription(attachment.user_id, source_id).await else {
        return edit_plain(bot, chat_id, message_id, lang.set_user_not_subscribed()).await;
    };
    render_and_edit_setting(bot, chat_id, message_id, &source, &sub, attachment, lang).await
}

async fn handle_toggle_update(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, chat_id.0).await;
    if !is_authorized(attachment, query) {
        return respond_toast(bot, query, lang.toast_error()).await;
    }
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(sub)) = state.repo.subscription(attachment.user_id, source_id).await else {
        return respond_toast(bot, query, lang.toast_error()).await;
    };
    let Ok(Some(source)) = state.repo.toggle_source_update_status(source_id).await else {
        return respond_toast(bot, query, lang.toast_error()).await;
    };
    respond_toast(bot, query, lang.set_modified_toast()).await?;
    render_and_edit_setting(bot, chat_id, message_id, &source, &sub, attachment, lang).await
}

async fn handle_toggle_notice(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, chat_id.0).await;
    if !is_authorized(attachment, query) {
        return edit_plain(bot, chat_id, message_id, lang.system_error_exclaim()).await;
    }
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(source)) = state.repo.get_source(source_id).await else {
        return respond_toast(bot, query, lang.toast_error()).await;
    };
    let Ok(Some(sub)) = state
        .repo
        .toggle_subscription_notice(attachment.user_id, source_id)
        .await
    else {
        return respond_toast(bot, query, lang.toast_error()).await;
    };
    respond_toast(bot, query, lang.set_modified_toast()).await?;
    render_and_edit_setting(bot, chat_id, message_id, &source, &sub, attachment, lang).await
}

async fn handle_toggle_telegraph(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, chat_id.0).await;
    if !is_authorized(attachment, query) {
        return respond_toast(bot, query, lang.toast_error()).await;
    }
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(source)) = state.repo.get_source(source_id).await else {
        return respond_toast(bot, query, lang.toast_error()).await;
    };
    let Ok(Some(sub)) = state
        .repo
        .toggle_subscription_telegraph(attachment.user_id, source_id)
        .await
    else {
        return respond_toast(bot, query, lang.toast_error()).await;
    };
    respond_toast(bot, query, lang.set_modified_toast()).await?;
    render_and_edit_setting(bot, chat_id, message_id, &source, &sub, attachment, lang).await
}

async fn handle_toggle_source_title(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, chat_id.0).await;
    if !is_authorized(attachment, query) {
        return respond_toast(bot, query, lang.toast_error()).await;
    }
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(source)) = state.repo.get_source(source_id).await else {
        return respond_toast(bot, query, lang.toast_error()).await;
    };
    let Ok(Some(sub)) = state
        .repo
        .toggle_subscription_source_title(attachment.user_id, source_id)
        .await
    else {
        return respond_toast(bot, query, lang.toast_error()).await;
    };
    respond_toast(bot, query, lang.set_modified_toast()).await?;
    render_and_edit_setting(bot, chat_id, message_id, &source, &sub, attachment, lang).await
}

// Go's `SetSubscriptionTagButton` replies with legacy `tb.ModeMarkdown`.
#[allow(deprecated)]
async fn handle_set_sub_tag(
    bot: &Bot,
    state: &BotState,
    query: &CallbackQuery,
    attachment: Attachment,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, chat_id.0).await;
    if !is_authorized(attachment, query) {
        // Go's `feedSetAuth` failure sends a *new* message via `ctx.Send`,
        // unlike every other handler here which edits in place.
        bot.send_message(chat_id, lang.set_tag_no_permission()).await?;
        return Ok(());
    }
    let source_id = attachment.source_id;
    let text = lang
        .set_tag_prompt()
        .replace("{source_id}", &source_id.to_string());
    bot.edit_message_text(chat_id, message_id, text)
        .parse_mode(ParseMode::Markdown)
        .await?;
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
    let lang = chat_lang(&state.repo, chat_id.0).await;
    let source_id = i64::from(attachment.source_id);
    let Ok(Some(source)) = state.repo.get_source(source_id).await else {
        return edit_plain(bot, chat_id, message_id, lang.unsub_error()).await;
    };
    match state
        .repo
        .unsubscribe_user(attachment.user_id, source_id)
        .await
    {
        Ok(_) => {
            let text = lang.unsub_succeeded_html(
                source_id,
                source.link.as_deref().unwrap_or(""),
                source.title.as_deref().unwrap_or(""),
            );
            bot.edit_message_text(chat_id, message_id, text)
                .parse_mode(ParseMode::Html)
                .await?;
            Ok(())
        }
        Err(_) => edit_plain(bot, chat_id, message_id, lang.unsub_error()).await,
    }
}

async fn handle_unsuball_confirm(
    bot: &Bot,
    state: &BotState,
    query: &CallbackQuery,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, chat_id.0).await;
    let sender_id = query.from.id.0 as i64;
    match state.repo.unsubscribe_all_user(sender_id).await {
        Ok(_) => edit_plain(bot, chat_id, message_id, lang.unsub_succeeded_plain()).await,
        Err(_) => edit_plain(bot, chat_id, message_id, lang.unsub_failed_plain()).await,
    }
}

async fn handle_unsuball_cancel(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    lang: Lang,
) -> ResponseResult<()> {
    edit_plain(bot, chat_id, message_id, lang.unsub_cancelled()).await
}
