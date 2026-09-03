use std::sync::Arc;

use teloxide::{
    prelude::*,
    types::{ChatId, LinkPreviewOptions, ParseMode},
};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::{
    bot::{
        auth,
        callbacks::handle_callback,
        commands::{Command, COMMAND_NAMES},
        documents::handle_document,
        html_format::compose_feed_message,
        keyboard::{feed_item_list_keyboard, settings_keyboard, unsuball_confirm_keyboard},
        sender::{MessageSender, SendOptions, TeloxideSender},
        subscribe::create_source,
    },
    config::Config,
    db::{models::Content, repo::Repo},
    feed::{
        fetch::{FetchOutcome, Fetcher},
        hash::gen_hash_id,
        parse::parse_feed,
    },
    opml::{export_opml, OpmlSource},
    preview::{PreviewPublisher, PublishRequest, TelegraphPublisher},
};

pub use crate::bot::i18n::Lang;

#[derive(Clone)]
pub struct BotState {
    pub repo: Repo,
    pub config: Config,
    pub fetcher: Fetcher,
}

/// Runs the Telegram long-polling dispatcher until `shutdown` fires, then
/// requests a graceful stop (sanctioned deviation D7): teloxide finishes the
/// in-flight update before `dispatch()` returns.
pub async fn run_bot(
    bot: Bot,
    config: Config,
    repo: Repo,
    fetcher: Fetcher,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    // The Telegram bot command menu is global per-bot, not per-chat, so we
    // register it once using the default locale. Per-chat language still
    // governs all in-chat replies via `chat_lang`.
    let default_lang = Lang::En;
    let mut commands = Vec::with_capacity(COMMAND_NAMES.len());
    for (name, description) in default_lang.command_descriptions().iter() {
        if !description.is_empty() {
            commands.push(teloxide::types::BotCommand::new(*name, *description));
        }
    }
    bot.set_my_commands(commands).await?;

    let state = Arc::new(BotState {
        repo,
        config,
        fetcher,
    });

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .filter(reject_unauthorized_message)
                .endpoint(handle_command),
        )
        .branch(
            Update::filter_message()
                .filter(|msg: Message| msg.document().is_some())
                .filter(reject_unauthorized_message)
                .endpoint(handle_document),
        )
        .branch(
            Update::filter_callback_query()
                .filter(reject_unauthorized_callback)
                .endpoint(handle_callback),
        );

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

/// `dptree` filter: allow the message through when `config.allowed_users` is
/// empty or contains the sender's Telegram user ID. Reject otherwise.
///
/// Channel posts and messages without a `from` field (`from` is `None` for
/// anonymous admins/owners) are treated as unauthorized: the bot is meant
/// for person-to-person subscriptions, so signing them off is safer than
/// silently accepting.
fn reject_unauthorized_message(state: Arc<BotState>, msg: Message) -> bool {
    match auth::message_user_id(&msg) {
        Some(user_id) => {
            if auth::is_allowed(&state.config, user_id) {
                true
            } else {
                warn!(
                    user_id,
                    chat_id = msg.chat.id.0,
                    allowed_users = ?state.config.allowed_users,
                    "dropped message: user not in allowed_users"
                );
                false
            }
        }
        None => {
            warn!(
                chat_id = msg.chat.id.0,
                "dropped message: no `from` field (channel post or anonymous admin)"
            );
            false
        }
    }
}

/// `dptree` filter for inline-button callbacks. Drop silently on rejection:
/// answering the toast would either leak that the bot is configured with an
/// allow-list, or update a message the rejecter is no longer looking at.
fn reject_unauthorized_callback(state: Arc<BotState>, query: CallbackQuery) -> bool {
    match auth::callback_user_id(&query) {
        Some(user_id) => {
            if auth::is_allowed(&state.config, user_id) {
                true
            } else {
                warn!(
                    user_id,
                    chat_instance = %query.chat_instance,
                    allowed_users = ?state.config.allowed_users,
                    "dropped callback query: user not in allowed_users"
                );
                false
            }
        }
        None => {
            warn!("dropped callback query: missing `from` field");
            false
        }
    }
}

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<BotState>,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    match cmd {
        Command::Start => {
            info!(chat_id = msg.chat.id.0, "/start");
            bot.send_message(msg.chat.id, lang.start_message()).await?;
        }
        Command::Ping => {
            bot.send_message(msg.chat.id, "pong").await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id, lang.help()).await?;
        }
        Command::Version => {
            bot.send_message(msg.chat.id, lang.version_message()).await?;
        }
        Command::List => list_subscriptions(&bot, &msg, &state, lang).await?,
        Command::Unsuball => {
            bot.send_message(msg.chat.id, lang.unsuball_confirm_prompt())
                .reply_markup(unsuball_confirm_keyboard(lang))
                .await?;
        }
        Command::Pauseall => set_all_sources_update(&bot, &msg, &state, false, lang).await?,
        Command::Activeall => set_all_sources_update(&bot, &msg, &state, true, lang).await?,
        Command::Sub(payload) => handle_subscribe(&bot, &msg, &state, payload.trim(), lang).await?,
        Command::Unsub(payload) => {
            handle_unsubscribe(&bot, &msg, &state, payload.trim(), lang).await?
        }
        Command::Setfeedtag(payload) => {
            handle_set_tag(&bot, &msg, &state, payload.trim(), lang).await?
        }
        Command::Set => handle_set(&bot, &msg, &state, lang).await?,
        Command::Settings => handle_settings(&bot, &msg, lang).await?,
        Command::Check => handle_check(&bot, &msg, &state, lang).await?,
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
    lang: Lang,
) -> ResponseResult<()> {
    if payload.is_empty() {
        bot.send_message(msg.chat.id, lang.sub_missing_url()).await?;
        return Ok(());
    }

    let source = match create_source(&state.repo, &state.fetcher, payload).await {
        Ok(source) => source,
        Err(err) => {
            bot.send_message(msg.chat.id, format!("{err} ({})", lang.sub_failed()))
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
            lang.sub_succeeded_md(
                source.id,
                source.title.as_deref().unwrap_or(payload),
                source.link.as_deref().unwrap_or(payload),
            ),
        )
        .parse_mode(ParseMode::Markdown)
        .link_preview_options(no_preview())
        .await?;
    } else {
        bot.send_message(msg.chat.id, lang.sub_already()).await?;
    }
    Ok(())
}

#[allow(deprecated)]
async fn handle_unsubscribe(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    payload: &str,
    lang: Lang,
) -> ResponseResult<()> {
    if payload.is_empty() {
        let sources = state
            .repo
            .subscriptions_for_user(msg.chat.id.0)
            .await
            .map_err(to_request_error)?;
        if sources.is_empty() {
            bot.send_message(msg.chat.id, lang.unsub_no_subs()).await?;
            return Ok(());
        }
        let items = sources
            .iter()
            .filter_map(|s| Some((s.source_id?, s.title.clone().unwrap_or_default())))
            .collect::<Vec<_>>();
        bot.send_message(msg.chat.id, lang.unsub_choose())
            .reply_markup(feed_item_list_keyboard(
                crate::bot::callback::Button::UnsubFeedItem,
                msg.chat.id.0,
                &items,
                lang,
            ))
            .await?;
        return Ok(());
    }

    match state
        .repo
        .source_by_link(payload)
        .await
        .map_err(to_request_error)?
    {
        None => {
            bot.send_message(msg.chat.id, lang.unsub_not_subscribed()).await?;
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
                    lang.unsub_succeeded_md(
                        source.title.as_deref().unwrap_or(""),
                        source.link.as_deref().unwrap_or(""),
                    ),
                )
                .parse_mode(ParseMode::Markdown)
                .link_preview_options(no_preview())
                .await?;
            } else {
                bot.send_message(msg.chat.id, lang.unsub_failed()).await?;
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
    lang: Lang,
) -> ResponseResult<()> {
    let mut parts = payload.split_whitespace();
    let Some(source_id) = parts.next().and_then(|s| s.parse::<i64>().ok()) else {
        bot.send_message(msg.chat.id, lang.setfeedtag_usage()).await?;
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
        bot.send_message(msg.chat.id, lang.setfeedtag_succeeded()).await?;
    } else {
        bot.send_message(msg.chat.id, lang.setfeedtag_failed()).await?;
    }
    Ok(())
}

/// Port of Go's `Set.Handle`: shows one inline button per subscribed source;
/// tapping one opens the toggle screen handled in `callbacks.rs`.
async fn handle_set(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    lang: Lang,
) -> ResponseResult<()> {
    let sources = state
        .repo
        .subscriptions_for_user(msg.chat.id.0)
        .await
        .map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(msg.chat.id, lang.set_no_subs()).await?;
        return Ok(());
    }
    let items = sources
        .iter()
        .filter_map(|s| Some((s.source_id?, s.title.clone().unwrap_or_default())))
        .collect::<Vec<_>>();
    bot.send_message(msg.chat.id, lang.set_choose())
        .reply_markup(feed_item_list_keyboard(
            crate::bot::callback::Button::SetFeedItem,
            msg.chat.id.0,
            &items,
            lang,
        ))
        .await?;
    Ok(())
}

async fn handle_settings(bot: &Bot, msg: &Message, lang: Lang) -> ResponseResult<()> {
    bot.send_message(msg.chat.id, lang.settings_title())
        .reply_markup(settings_keyboard(lang))
        .await?;
    Ok(())
}

async fn handle_check(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    lang: Lang,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;
    let sources = state
        .repo
        .subscriptions_for_user(chat_id)
        .await
        .map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(msg.chat.id, lang.set_no_subs()).await?;
        return Ok(());
    }

    bot.send_message(
        msg.chat.id,
        lang.check_started_msg(sources.len()),
    )
    .await?;

    let sender = TeloxideSender::new(bot.clone());
    let publisher = TelegraphPublisher::new(&state.config.telegraph_token);
    let mut new_count = 0usize;
    let mut unchanged_count = 0usize;
    let mut error_count = 0usize;
    let now = now_unix();

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
                        .insert_content(&Content {
                            source_id: Some(source.id),
                            hash_id: hash_id.clone(),
                            raw_id: Some(item.guid.clone()),
                            raw_link: Some(item.link.clone()),
                            title: Some(item.title.clone()),
                            telegraph_url: telegraph_url.clone(),
                            created_at: None,
                            updated_at: None,
                        })
                        .await
                        .map_err(to_request_error)?;

                    let description_html = item
                        .content
                        .as_deref()
                        .or(item.description.as_deref())
                        .unwrap_or("");
                    let enable_telegraph =
                        sub.enable_telegraph == Some(1) && telegraph_url.is_some();
                    let composed = compose_feed_message(
                        source.title.as_deref().unwrap_or(""),
                        &item.title,
                        &item.link,
                        sub.tag.as_deref().unwrap_or(""),
                        description_html,
                        if enable_telegraph { telegraph_url.as_deref() } else { None },
                    );
                    let _ = sender
                        .send_text(
                            chat_id,
                            &composed,
                            SendOptions {
                                disable_web_page_preview: state.config.disable_web_page_preview,
                                disable_notification: sub.enable_notification != Some(1),
                            },
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
                    .mark_source_error(source.id, now + 60)
                    .await
                    .map_err(to_request_error)?;
            }
        }
    }

    bot.send_message(
        msg.chat.id,
        lang.check_done_msg(new_count, unchanged_count, error_count),
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
    lang: Lang,
) -> ResponseResult<()> {
    let sources = match state.repo.subscriptions_for_user(msg.chat.id.0).await {
        Ok(sources) => sources,
        Err(_) => {
            bot.send_message(msg.chat.id, lang.system_error()).await?;
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
            let text = if enable {
                lang.activeall_failed()
            } else {
                lang.pauseall_failed()
            };
            bot.send_message(msg.chat.id, text).await?;
            return Ok(());
        }
    }
    let reply = if enable {
        lang.activeall_succeeded()
    } else {
        lang.pauseall_succeeded()
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
    lang: Lang,
) -> ResponseResult<()> {
    let sources = state
        .repo
        .subscriptions_for_user(owner_id)
        .await
        .map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(chat_id, lang.opml_export_empty()).await?;
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
        bot.send_message(chat_id, lang.opml_export_failed()).await?;
        return Ok(());
    };

    let file_name = format!("subscriptions_{}.opml", now_unix());
    let document = teloxide::types::InputFile::memory(opml_text.into_bytes()).file_name(file_name);
    if bot.send_document(chat_id, document).await.is_err() {
        bot.send_message(chat_id, lang.opml_export_failed()).await?;
    }
    Ok(())
}

#[allow(deprecated)]
async fn list_subscriptions(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    lang: Lang,
) -> ResponseResult<()> {
    let sources = state
        .repo
        .subscriptions_for_user(msg.chat.id.0)
        .await
        .map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(msg.chat.id, lang.list_empty()).await?;
        return Ok(());
    }
    let mut text = lang.list_header(sources.len());
    for source in sources {
        text.push_str(&format!(
            "[[{}]] [{}]({})\n",
            source.source_id.unwrap_or_default(),
            source.title.unwrap_or_default(),
            source.link.unwrap_or_default()
        ));
    }
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
    repo.set_option(&lang_option_name(chat_id), lang.value()).await?;
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

fn now_unix() -> i64 {
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
