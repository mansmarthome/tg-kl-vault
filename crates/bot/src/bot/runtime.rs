use std::sync::Arc;

use teloxide::{
    prelude::*,
    types::{ChatId, ForceReply, InlineKeyboardButton, InlineKeyboardMarkup, LinkPreviewOptions, ParseMode},
    utils::command::BotCommands,
};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::{
    bot::{
        bookmarks,
        broadcast::{send_item_to_chat, ItemForChat, SubOptions},
        callbacks::handle_callback,
        commands::Command,
        documents::handle_document,
        feedcheck,
        keyboard::{feed_item_list_keyboard, settings_keyboard, unsuball_confirm_keyboard},
        sender::TeloxideSender,
        stocks,
        subscribe::create_source,
    },
    config::Config,
    db::repo::Repo,
    feed::{
        fetch::{FetchOutcome, Fetcher},
        hash::gen_hash_id,
        parse::{is_stale_item, parse_feed},
    },
    opml::{export_opml, OpmlSource},
    preview::{PreviewPublisher, PublishRequest, TelegraphPublisher},
    scheduler::ledger_entry,
};

pub use crate::bot::i18n::Lang;

#[derive(Clone)]
pub struct BotState {
    pub repo: Repo,
    pub config: Config,
    pub fetcher: Fetcher,
    /// Shared with the stock worker so the rate limiter / 429 cooldown / hard
    /// lock and the interactive path see the same state.
    pub stock: Arc<crate::bot::stocks::StockSvc>,
}

/// Runs the Telegram long-polling dispatcher until `shutdown` fires, then
/// requests a graceful stop (sanctioned deviation D7): teloxide finishes the
/// in-flight update before `dispatch()` returns.
pub async fn run_bot(
    bot: Bot,
    config: Config,
    repo: Repo,
    fetcher: Fetcher,
    stock: Arc<crate::bot::stocks::StockSvc>,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    // Registered menu = the derive-generated list, minus deliberately hidden
    // entries (empty description, e.g. /ping). Telegram rejects empty
    // descriptions, and this keeps "add a command" a one-place change instead
    // of "edit three places and break one test".
    let commands = Command::bot_commands()
        .into_iter()
        .filter(|command| !command.description.is_empty())
        .collect::<Vec<_>>();
    bot.set_my_commands(commands).await?;

    let state = Arc::new(BotState {
        repo,
        config,
        fetcher,
        stock,
    });

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(
            Update::filter_message()
                .filter(|msg: Message| msg.document().is_some())
                .endpoint(handle_document),
        )
        .branch(
            Update::filter_message()
                .filter(|msg: Message| msg.text().is_some() && msg.reply_to_message().is_some())
                .endpoint(handle_prompt_reply),
        )
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .build();
    let shutdown_token = dispatcher.shutdown_token();

    let dispatch_task = tokio::spawn(async move { dispatcher.dispatch().await });

    if shutdown.changed().await.is_ok() && *shutdown.borrow() {
        if let Ok(done) = shutdown_token.shutdown() {
            done.await;
        }
    }
    dispatch_task.await?;
    Ok(())
}

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<BotState>,
) -> ResponseResult<()> {
    match cmd {
        Command::Start => {
            info!(chat_id = msg.chat.id.0, "/start");
            bot.send_message(msg.chat.id, "你好，歡迎使用 flowerss。")
                .await?;
        }
        Command::Ping => {
            bot.send_message(msg.chat.id, "pong").await?;
        }
        Command::Help => {
            let lang = chat_lang(&state.repo, msg.chat.id.0).await;
            bot.send_message(msg.chat.id, lang.help()).await?;
        }
        Command::Version => {
            bot.send_message(msg.chat.id, "tg-kl-vault compatible with flowerss-bot, version dev, commit none, built at unknown").await?;
        }
        Command::List => list_subscriptions(&bot, &msg, &state).await?,
        Command::Unsuball => {
            bot.send_message(msg.chat.id, "是否退訂目前使用者的所有訂閱？")
                .reply_markup(unsuball_confirm_keyboard())
                .await?;
        }
        Command::Pauseall => set_all_sources_update(&bot, &msg, &state, false).await?,
        Command::Activeall => set_all_sources_update(&bot, &msg, &state, true).await?,
        Command::Sub(payload) => handle_subscribe(&bot, &msg, &state, payload.trim()).await?,
        Command::Unsub(payload) => handle_unsubscribe(&bot, &msg, &state, payload.trim()).await?,
        Command::Setfeedtag(payload) => handle_set_tag(&bot, &msg, &state, payload.trim()).await?,
        Command::Set => handle_set(&bot, &msg, &state).await?,
        Command::Settings => handle_settings(&bot, &msg, &state).await?,
        Command::Check => handle_check(&bot, &msg, &state).await?,
        Command::Feedcheck => feedcheck::handle_feedcheck(&bot, &msg, &state).await?,
        Command::Bm(payload) => bookmarks::handle_bm(&bot, &msg, &state, payload.trim()).await?,
        Command::Bookmarks => bookmarks::handle_bookmarks(&bot, &msg, &state).await?,
        Command::Bmsearch(payload) => {
            bookmarks::handle_bmsearch(&bot, &msg, &state, payload.trim()).await?
        }
        Command::Bmnote(payload) => {
            bookmarks::handle_bmnote(&bot, &msg, &state, payload.trim()).await?
        }
        Command::Bmtag(payload) => bookmarks::handle_bmtag(&bot, &msg, &state, &payload).await?,
        Command::Bmdel(payload) => {
            bookmarks::handle_bmdel(&bot, &msg, &state, payload.trim()).await?
        }
        Command::Stock(payload) => stocks::handle_stock(&bot, &msg, &state, payload.trim()).await?,
        Command::Stocks => stocks::handle_stocks(&bot, &msg, &state).await?,
        Command::Stockadd(payload) => {
            stocks::handle_stockadd(&bot, &msg, &state, payload.trim()).await?
        }
        Command::Stockpush(payload) => {
            stocks::handle_stockpush(&bot, &msg, &state, payload.trim()).await?
        }
        Command::Stockreport => stocks::handle_stockreport(&bot, &msg, &state).await?,
        Command::Stockdel(payload) => {
            stocks::handle_stockdel(&bot, &msg, &state, payload.trim()).await?
        }
    }
    Ok(())
}

pub const PROMPT_CANCEL_PREFIX: &str = "prompt:cancel:";

fn force_reply(placeholder: &str) -> ForceReply {
    ForceReply::new().input_field_placeholder(placeholder.to_owned())
}

pub async fn send_force_reply_prompt(
    bot: &Bot,
    chat_id: ChatId,
    lang: Lang,
    prompt: &str,
    placeholder: &str,
) -> ResponseResult<()> {
    let prompt_message = bot
        .send_message(chat_id, prompt)
        .reply_markup(force_reply(placeholder))
        .await?;
    bot.send_message(chat_id, lang.prompt_cancel_control())
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
            lang.prompt_cancel_button(),
            format!("{PROMPT_CANCEL_PREFIX}{}", prompt_message.id.0),
        )]]))
        .await?;
    Ok(())
}

fn is_cancel(payload: &str) -> bool {
    matches!(payload.trim().to_ascii_lowercase().as_str(), "取消" | "cancel" | "/cancel")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    Sub,
    SetFeedTag,
    Bm,
    BmSearch,
    BmNote,
    BmTag,
    BmDel,
    Stock,
    StockAdd,
    StockDel,
    // Reply routing is stateless — the only state a reply carries is the text
    // of the message it replied to. So "set the push time" can't be one prompt
    // with a market argument; it must be two distinct prompt strings mapping to
    // two kinds. Any attempt to stash "which market" in a side table would
    // reintroduce the per-chat prompt state this design deletes.
    StockPushTimeTw,
    StockPushTimeUs,
}

fn prompt_kind(text: &str) -> Option<PromptKind> {
    for lang in [Lang::En, Lang::ZhTw] {
        if text == lang.sub_prompt() {
            return Some(PromptKind::Sub);
        }
        if text == lang.setfeedtag_prompt() {
            return Some(PromptKind::SetFeedTag);
        }
        if text == lang.bm_prompt() {
            return Some(PromptKind::Bm);
        }
        if text == lang.bm_search_prompt() {
            return Some(PromptKind::BmSearch);
        }
        if text == lang.bm_note_prompt() {
            return Some(PromptKind::BmNote);
        }
        if text == lang.bm_tag_prompt() {
            return Some(PromptKind::BmTag);
        }
        if text == lang.bm_delete_prompt() {
            return Some(PromptKind::BmDel);
        }
        if text == lang.stk_prompt() {
            return Some(PromptKind::Stock);
        }
        if text == lang.stk_add_prompt() {
            return Some(PromptKind::StockAdd);
        }
        if text == lang.stk_del_prompt() {
            return Some(PromptKind::StockDel);
        }
        if text == lang.stk_push_time_tw_prompt() {
            return Some(PromptKind::StockPushTimeTw);
        }
        if text == lang.stk_push_time_us_prompt() {
            return Some(PromptKind::StockPushTimeUs);
        }
    }
    None
}

async fn handle_prompt_reply(bot: Bot, msg: Message, state: Arc<BotState>) -> ResponseResult<()> {
    let replied_to = msg
        .reply_to_message()
        .and_then(|m| m.text())
        .unwrap_or_default();
    let payload = msg.text().unwrap_or_default().trim().to_owned();
    let Some(kind) = prompt_kind(replied_to) else {
        return Ok(());
    };

    if is_cancel(&payload) {
        let lang = chat_lang(&state.repo, msg.chat.id.0).await;
        bot.send_message(msg.chat.id, lang.prompt_cancelled()).await?;
        return Ok(());
    }

    match kind {
        PromptKind::Sub => handle_subscribe(&bot, &msg, &state, &payload).await?,
        PromptKind::SetFeedTag => handle_set_tag(&bot, &msg, &state, &payload).await?,
        PromptKind::Bm => bookmarks::handle_bm(&bot, &msg, &state, &payload).await?,
        PromptKind::BmSearch => bookmarks::handle_bmsearch(&bot, &msg, &state, &payload).await?,
        PromptKind::BmNote => bookmarks::handle_bmnote(&bot, &msg, &state, &payload).await?,
        PromptKind::BmTag => bookmarks::handle_bmtag(&bot, &msg, &state, &payload).await?,
        PromptKind::BmDel => bookmarks::handle_bmdel(&bot, &msg, &state, &payload).await?,
        PromptKind::Stock => stocks::handle_stock(&bot, &msg, &state, &payload).await?,
        PromptKind::StockAdd => stocks::handle_stockadd(&bot, &msg, &state, &payload).await?,
        PromptKind::StockDel => stocks::handle_stockdel(&bot, &msg, &state, &payload).await?,
        PromptKind::StockPushTimeTw => {
            stocks::handle_push_time_reply(&bot, &msg, &state, crate::stock::Market::Tw, &payload).await?
        }
        PromptKind::StockPushTimeUs => {
            stocks::handle_push_time_reply(&bot, &msg, &state, crate::stock::Market::Us, &payload).await?
        }
    }
    Ok(())
}

// Go sends these replies with legacy `tb.ModeMarkdown`, not MarkdownV2 (which
// would require escaping titles/tags for punctuation Go never escapes).
#[allow(deprecated)]
async fn handle_subscribe(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    payload: &str,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    if payload.is_empty() {
        send_force_reply_prompt(
            bot,
            msg.chat.id,
            lang,
            lang.sub_prompt(),
            lang.sub_placeholder(),
        )
        .await?;
        return Ok(());
    }

    let source = match create_source(&state.repo, &state.fetcher, payload).await {
        Ok(source) => source,
        Err(err) => {
            bot.send_message(msg.chat.id, lang.sub_failed_retry(&err.to_string()))
                .await?;
            send_force_reply_prompt(
                bot,
                msg.chat.id,
                lang,
                lang.sub_prompt(),
                lang.sub_placeholder(),
            )
            .await?;
            return Ok(());
        }
    };

    if state
        .repo
        .subscribe_user(msg.chat.id.0, source.id)
        .await
        .map_err(to_request_error)?
    {
        bot.send_message(
            msg.chat.id,
            format!(
                "[[{}]][{}]({}) 訂閱成功",
                source.id,
                source.title.as_deref().unwrap_or(payload),
                source.link.as_deref().unwrap_or(payload)
            ),
        )
        .parse_mode(ParseMode::Markdown)
        .link_preview_options(no_preview())
        .await?;
    } else {
        bot.send_message(msg.chat.id, "已訂閱該源，請勿重複訂閱")
            .await?;
    }
    Ok(())
}

#[allow(deprecated)]
async fn handle_unsubscribe(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    payload: &str,
) -> ResponseResult<()> {
    if payload.is_empty() {
        let sources = state
            .repo
            .subscriptions_for_user(msg.chat.id.0)
            .await
            .map_err(to_request_error)?;
        if sources.is_empty() {
            bot.send_message(msg.chat.id, "沒有訂閱").await?;
            return Ok(());
        }
        let items = sources
            .iter()
            .filter_map(|s| Some((s.source_id?, s.title.clone().unwrap_or_default())))
            .collect::<Vec<_>>();
        bot.send_message(msg.chat.id, "請選擇你要退訂的源")
            .reply_markup(feed_item_list_keyboard(
                crate::bot::callback::Button::UnsubFeedItem,
                msg.chat.id.0,
                &items,
            ))
            .await?;
        return Ok(());
    }

    match state
        .repo
        .source_for_unsub_arg(payload)
        .await
        .map_err(to_request_error)?
    {
        None => {
            bot.send_message(msg.chat.id, "未訂閱該 RSS 源").await?;
        }
        Some(source) => {
            if state
                .repo
                .unsubscribe_user(msg.chat.id.0, source.id)
                .await
                .map_err(to_request_error)?
            {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "[{}]({}) 退訂成功！",
                        source.title.as_deref().unwrap_or(""),
                        source.link.as_deref().unwrap_or("")
                    ),
                )
                .parse_mode(ParseMode::Markdown)
                .link_preview_options(no_preview())
                .await?;
            } else {
                bot.send_message(msg.chat.id, "退訂失敗").await?;
            }
        }
    }
    Ok(())
}

async fn handle_set_tag(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    payload: &str,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    let mut parts = payload.split_whitespace();
    let Some(source_id) = parts.next().and_then(|s| s.parse::<i64>().ok()) else {
        send_force_reply_prompt(
            bot,
            msg.chat.id,
            lang,
            lang.setfeedtag_prompt(),
            lang.setfeedtag_placeholder(),
        )
        .await?;
        return Ok(());
    };
    // Go: `subscription.Tag = "#" + strings.Join(tags, " #")` — note this
    // yields the literal tag "#" when no tags are given, which we replicate.
    let tag = parts.take(3).collect::<Vec<_>>().join(" ");
    let tag = format!("#{}", tag.replace(' ', " #"));
    if state
        .repo
        .set_subscription_tag(msg.chat.id.0, source_id, &tag)
        .await
        .map_err(to_request_error)?
    {
        bot.send_message(msg.chat.id, "訂閱標籤設定成功!").await?;
    } else {
        bot.send_message(msg.chat.id, "訂閱標籤設定失敗!").await?;
    }
    Ok(())
}

/// Port of Go's `Set.Handle`: shows one inline button per subscribed source;
/// tapping one opens the toggle screen handled in `callbacks.rs`.
async fn handle_set(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let sources = state
        .repo
        .subscriptions_for_user(msg.chat.id.0)
        .await
        .map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(msg.chat.id, "目前沒有訂閱").await?;
        return Ok(());
    }
    let items = sources
        .iter()
        .filter_map(|s| Some((s.source_id?, s.title.clone().unwrap_or_default())))
        .collect::<Vec<_>>();
    bot.send_message(msg.chat.id, "請選擇你要設定的源")
        .reply_markup(feed_item_list_keyboard(
            crate::bot::callback::Button::SetFeedItem,
            msg.chat.id.0,
            &items,
        ))
        .await?;
    Ok(())
}

async fn handle_settings(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    bot.send_message(msg.chat.id, lang.settings_title())
        .reply_markup(settings_keyboard(lang))
        .await?;
    Ok(())
}

async fn handle_check(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;
    let sources = state
        .repo
        .subscriptions_for_user(chat_id)
        .await
        .map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(msg.chat.id, "目前沒有訂閱").await?;
        return Ok(());
    }

    bot.send_message(
        msg.chat.id,
        format!("已開始檢查目前訂閱，共{}個源", sources.len()),
    )
    .await?;

    let sender = TeloxideSender::new(bot.clone());
    let publisher = TelegraphPublisher::new(&state.config.telegraph_token);
    let mut new_count = 0usize;
    let mut stale_count = 0usize;
    let mut unchanged_count = 0usize;
    let mut error_count = 0usize;
    let now = now_unix();
    let bm_off = state
        .repo
        .chat_ids_with_option_off(crate::bot::bookmarks::BM_BTN_PREFIX)
        .await
        .unwrap_or_default();
    let summary_configured = state.config.bookmark.ai.mcp.is_configured();
    let sum_off = if summary_configured {
        state
            .repo
            .chat_ids_with_option_off(crate::bot::bookmarks::BM_SUM_PREFIX)
            .await
            .unwrap_or_default()
    } else {
        std::collections::HashSet::new()
    };

    for sub in sources {
        let Some(source_id) = sub.source_id else {
            continue;
        };
        let source = match state
            .repo
            .get_source(source_id)
            .await
            .map_err(to_request_error)?
        {
            Some(source) => source,
            None => continue,
        };
        let Some(link) = source.link.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };

        match state
            .fetcher
            .fetch(
                link,
                source.etag.as_deref(),
                source.last_modified.as_deref(),
            )
            .await
        {
            Ok(FetchOutcome::Unchanged) => {
                unchanged_count += 1;
                state
                    .repo
                    .mark_source_success(
                        source.id,
                        None,
                        None,
                        next_fetch_at(now, state.config.update_interval),
                    )
                    .await
                    .map_err(to_request_error)?;
            }
            Ok(FetchOutcome::Modified(feed)) => {
                let parsed = match parse_feed(&feed.body) {
                    Ok(parsed) => parsed,
                    Err(err) => {
                        warn!(source_id, error = %err, "manual check parse failed");
                        error_count += 1;
                        state
                            .repo
                            .mark_source_error(
                                source.id,
                                now + 60,
                                &format!("parse failed: {err}"),
                            )
                            .await
                            .map_err(to_request_error)?;
                        continue;
                    }
                };
                let hashes = parsed
                    .items
                    .iter()
                    .map(|item| gen_hash_id(link, &item.guid))
                    .collect::<Vec<_>>();
                let existing = state
                    .repo
                    .existing_hash_ids(source.id, &hashes)
                    .await
                    .map_err(to_request_error)?;

                for (item, hash_id) in parsed.items.iter().zip(hashes) {
                    if existing.contains(&hash_id) {
                        continue;
                    }
                    // Same age gate as the scheduler: `/check` force-fetches
                    // every subscription including paused and long-broken ones,
                    // so without this it is the most reliable way to dump a
                    // feed's entire back catalogue into the chat.
                    if is_stale_item(item.published, now, state.config.fetch.max_item_age_days) {
                        info!(
                            source_id,
                            hash_id = %hash_id,
                            published = ?item.published,
                            "manual check skipping stale item"
                        );
                        state
                            .repo
                            .insert_content(&ledger_entry(source.id, item, &hash_id, None))
                            .await
                            .map_err(to_request_error)?;
                        stale_count += 1;
                        continue;
                    }

                    let telegraph_url = publisher
                        .publish(&PublishRequest {
                            title: &item.title,
                            author_name: Some(&state.config.telegraph_author_name),
                            author_url: non_empty(&state.config.telegraph_author_url),
                            html: item.content.as_deref().or(item.description.as_deref()).unwrap_or(""),
                            base_url: Some(&item.link),
                        })
                        .await
                        .unwrap_or_else(|err| {
                            warn!(source_id, %hash_id, error = %err, "manual check telegraph publish failed");
                            None
                        });

                    state
                        .repo
                        .insert_content(&ledger_entry(
                            source.id,
                            item,
                            &hash_id,
                            telegraph_url.clone(),
                        ))
                        .await
                        .map_err(to_request_error)?;

                    let item_data = ItemForChat {
                        source_title: source.title.as_deref().unwrap_or(""),
                        content_title: &item.title,
                        raw_link: &item.link,
                        description: item.description.as_deref().unwrap_or(""),
                        telegraph_url: telegraph_url.as_deref(),
                        hash_id: &hash_id,
                    };
                    let sub_opts = SubOptions {
                        enable_notification: sub.enable_notification == Some(1),
                        enable_telegraph: sub.enable_telegraph == Some(1),
                        tag: sub.tag.as_deref().unwrap_or(""),
                    };
                    let bookmark_button = !bm_off.contains(&chat_id);
                    let summary_button = summary_configured && !sum_off.contains(&chat_id);
                    let _ = send_item_to_chat(
                        &sender,
                        &state.config,
                        chat_id,
                        &item_data,
                        &sub_opts,
                        bookmark_button,
                        summary_button,
                    )
                    .await;
                    new_count += 1;
                }

                state
                    .repo
                    .mark_source_success(
                        source.id,
                        feed.etag.as_deref(),
                        feed.last_modified.as_deref(),
                        next_fetch_at(now, state.config.update_interval),
                    )
                    .await
                    .map_err(to_request_error)?;
            }
            Err(err) => {
                warn!(source_id, error = %err, "manual check fetch failed");
                error_count += 1;
                state
                    .repo
                    .mark_source_error(source.id, now + 60, &err.to_string())
                    .await
                    .map_err(to_request_error)?;
            }
        }
    }

    bot.send_message(
        msg.chat.id,
        format!(
            "檢查完成：新增{}篇，忽略{}篇過舊，{}個源無更新，{}個源失敗",
            new_count, stale_count, unchanged_count, error_count
        ),
    )
    .await?;
    Ok(())
}

/// Port of Go's `PauseAll`/`ActiveAll`: these pause/resume the *source*
/// (`error_count`) for every source the caller is subscribed to, not a
/// per-subscriber flag — see `Core.DisableSourceUpdate`/`EnableSourceUpdate`.
#[allow(deprecated)]
async fn set_all_sources_update(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    enable: bool,
) -> ResponseResult<()> {
    let sources = match state.repo.subscriptions_for_user(msg.chat.id.0).await {
        Ok(sources) => sources,
        Err(_) => {
            bot.send_message(msg.chat.id, "系統錯誤").await?;
            return Ok(());
        }
    };
    for source in &sources {
        let Some(source_id) = source.source_id else {
            continue;
        };
        let result = if enable {
            state.repo.enable_source_update(source_id).await
        } else {
            state.repo.disable_source_update(source_id).await
        };
        if result.is_err() {
            bot.send_message(
                msg.chat.id,
                if enable {
                    "啟用失敗"
                } else {
                    "暫停失敗"
                },
            )
            .await?;
            return Ok(());
        }
    }
    let reply = if enable {
        "訂閱已全部開啟"
    } else {
        "訂閱已全部暫停"
    };
    bot.send_message(msg.chat.id, reply)
        .parse_mode(ParseMode::Markdown)
        .link_preview_options(no_preview())
        .await?;
    Ok(())
}

pub async fn export_chat_opml(
    bot: &Bot,
    chat_id: ChatId,
    owner_id: i64,
    state: &BotState,
) -> ResponseResult<()> {
    let sources = state
        .repo
        .subscriptions_for_user(owner_id)
        .await
        .map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(chat_id, "訂閱列表為空").await?;
        return Ok(());
    }
    let opml_sources = sources
        .iter()
        .map(|s| OpmlSource {
            title: s.title.clone().unwrap_or_default(),
            xml_url: s.link.clone().unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let Ok(opml_text) = export_opml(&opml_sources) else {
        bot.send_message(chat_id, "匯出失敗").await?;
        return Ok(());
    };

    let file_name = format!("subscriptions_{}.opml", now_unix());
    let document = teloxide::types::InputFile::memory(opml_text.into_bytes()).file_name(file_name);
    if bot.send_document(chat_id, document).await.is_err() {
        bot.send_message(chat_id, "匯出失敗").await?;
    }
    Ok(())
}

#[allow(deprecated)]
async fn list_subscriptions(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let sources = state
        .repo
        .subscriptions_for_user(msg.chat.id.0)
        .await
        .map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(msg.chat.id, "訂閱列表為空").await?;
        return Ok(());
    }
    let mut text = format!("共訂閱{}個源，訂閱列表\n", sources.len());
    for source in sources {
        // A feed the scheduler gave up on used to look identical to a healthy
        // one here; the marker is the cheapest place to notice it.
        let marker = if source.is_paused() {
            "⏸ "
        } else if source.error_count.unwrap_or(0) > 0 {
            "⚠️ "
        } else {
            "✅ "
        };
        text.push_str(&format!(
            "{}[[{}]] [{}]({})\n",
            marker,
            source.source_id.unwrap_or_default(),
            source.title.unwrap_or_default(),
            source.link.unwrap_or_default()
        ));
    }
    text.push_str("\n⏸ 已暫停／⚠️ 抓取失敗中，用 /feedcheck 查看詳情");
    bot.send_message(msg.chat.id, text)
        .parse_mode(ParseMode::Markdown)
        .link_preview_options(no_preview())
        .await?;
    Ok(())
}

fn lang_option_name(chat_id: i64) -> String {
    format!("tg-kl-vault:lang:{chat_id}")
}

pub async fn chat_lang(repo: &Repo, chat_id: i64) -> Lang {
    Lang::from_value(
        repo.get_option(&lang_option_name(chat_id))
            .await
            .ok()
            .flatten()
            .as_deref(),
    )
}

pub async fn set_chat_lang(repo: &Repo, chat_id: i64, lang: Lang) -> anyhow::Result<()> {
    repo.set_option(&lang_option_name(chat_id), lang.value())
        .await?;
    Ok(())
}

pub(crate) fn no_preview() -> LinkPreviewOptions {
    LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    }
}

pub(crate) fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn next_fetch_at(now: i64, interval_minutes: u64) -> i64 {
    now + interval_minutes.max(1) as i64 * 60
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

pub(crate) fn to_request_error(
    err: impl std::error::Error + Send + Sync + 'static,
) -> teloxide::RequestError {
    teloxide::RequestError::Io(std::sync::Arc::new(std::io::Error::other(err)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reply routing is stateless: a reply is matched back to a `PromptKind`
    /// purely by the text of the message it replied to. So every prompt string
    /// must map to *some* kind, and no two prompts (in either language) may
    /// share text — a copy-pasted prompt would silently misroute replies to the
    /// wrong handler. This guards a gap the other tests didn't cover.
    #[test]
    fn every_prompt_string_maps_to_exactly_one_prompt_kind() {
        let mut seen = std::collections::HashSet::new();
        for lang in [Lang::En, Lang::ZhTw] {
            let prompts = [
                lang.sub_prompt(),
                lang.setfeedtag_prompt(),
                lang.bm_prompt(),
                lang.bm_search_prompt(),
                lang.bm_note_prompt(),
                lang.bm_tag_prompt(),
                lang.bm_delete_prompt(),
                lang.stk_prompt(),
                lang.stk_add_prompt(),
                lang.stk_del_prompt(),
                lang.stk_push_time_tw_prompt(),
                lang.stk_push_time_us_prompt(),
            ];
            for p in prompts {
                assert!(prompt_kind(p).is_some(), "prompt not routed: {p:?}");
                assert!(seen.insert(p.to_owned()), "duplicate prompt text misroutes: {p:?}");
            }
        }
    }
}
