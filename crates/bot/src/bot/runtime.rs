use std::sync::Arc;

use teloxide::{prelude::*, types::{LinkPreviewOptions, ParseMode}};
use tokio::sync::watch;
use tracing::info;

use crate::{
    bot::{
        callbacks::handle_callback,
        commands::{Command, COMMANDS},
        documents::handle_document,
        keyboard::{feed_item_list_keyboard, unsuball_confirm_keyboard},
        subscribe::create_source,
    },
    config::Config,
    db::repo::Repo,
    feed::fetch::Fetcher,
    opml::{export_opml, OpmlSource},
};

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
    let commands = COMMANDS
        .iter()
        .filter(|(_, description)| !description.is_empty())
        .map(|(command, description)| teloxide::types::BotCommand::new(*command, *description))
        .collect::<Vec<_>>();
    bot.set_my_commands(commands).await?;

    let state = Arc::new(BotState { repo, config, fetcher });

    let handler = dptree::entry()
        .branch(Update::filter_message().filter_command::<Command>().endpoint(handle_command))
        .branch(
            Update::filter_message()
                .filter(|msg: Message| msg.document().is_some())
                .endpoint(handle_document),
        )
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    let mut dispatcher =
        Dispatcher::builder(bot, handler).dependencies(dptree::deps![state]).build();
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

async fn handle_command(bot: Bot, msg: Message, cmd: Command, state: Arc<BotState>) -> ResponseResult<()> {
    match cmd {
        Command::Start => {
            info!(chat_id = msg.chat.id.0, "/start");
            bot.send_message(msg.chat.id, "你好，欢迎使用flowerss。").await?;
        }
        Command::Ping => {
            bot.send_message(msg.chat.id, "pong").await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id, help_message()).await?;
        }
        Command::Version => {
            bot.send_message(msg.chat.id, "version dev, commit none, built at unknown").await?;
        }
        Command::List => list_subscriptions(&bot, &msg, &state).await?,
        Command::Unsuball => {
            bot.send_message(msg.chat.id, "是否退订当前用户的所有订阅？")
                .reply_markup(unsuball_confirm_keyboard())
                .await?;
        }
        Command::Pauseall => set_all_sources_update(&bot, &msg, &state, false).await?,
        Command::Activeall => set_all_sources_update(&bot, &msg, &state, true).await?,
        Command::Sub(payload) => handle_subscribe(&bot, &msg, &state, payload.trim()).await?,
        Command::Unsub(payload) => handle_unsubscribe(&bot, &msg, &state, payload.trim()).await?,
        Command::Setfeedtag(payload) => handle_set_tag(&bot, &msg, &state, payload.trim()).await?,
        Command::Setinterval(payload) => handle_set_interval(&bot, &msg, &state, payload.trim()).await?,
        Command::Set => handle_set(&bot, &msg, &state).await?,
        Command::Check => handle_check(&bot, &msg, &state).await?,
        Command::Import => {
            bot.send_message(
                msg.chat.id,
                "请直接发送OPML文件，如果需要为频道导入OPML，请在发送文件的时候附上channel id，例如@telegram",
            )
            .await?;
        }
        Command::Export => handle_export(&bot, &msg, &state).await?,
    }
    Ok(())
}

// Go sends these replies with legacy `tb.ModeMarkdown`, not MarkdownV2 (which
// would require escaping titles/tags for punctuation Go never escapes).
#[allow(deprecated)]
async fn handle_subscribe(bot: &Bot, msg: &Message, state: &BotState, payload: &str) -> ResponseResult<()> {
    if payload.is_empty() {
        bot.send_message(
            msg.chat.id,
            "请在命令后带上需要订阅的RSS URL，例如：/sub https://justinpot.com/feed/",
        )
        .await?;
        return Ok(());
    }

    let source = match create_source(&state.repo, &state.fetcher, payload).await {
        Ok(source) => source,
        Err(err) => {
            bot.send_message(msg.chat.id, format!("{err}，订阅失败")).await?;
            return Ok(());
        }
    };

    if state.repo.subscribe_user(msg.chat.id.0, source.id).await.map_err(to_request_error)? {
        bot.send_message(
            msg.chat.id,
            format!(
                "[[{}]][{}]({}) 订阅成功",
                source.id,
                source.title.as_deref().unwrap_or(payload),
                source.link.as_deref().unwrap_or(payload)
            ),
        )
        .parse_mode(ParseMode::Markdown)
        .link_preview_options(no_preview())
        .await?;
    } else {
        bot.send_message(msg.chat.id, "已订阅该源，请勿重复订阅").await?;
    }
    Ok(())
}

#[allow(deprecated)]
async fn handle_unsubscribe(bot: &Bot, msg: &Message, state: &BotState, payload: &str) -> ResponseResult<()> {
    if payload.is_empty() {
        let sources = state.repo.subscriptions_for_user(msg.chat.id.0).await.map_err(to_request_error)?;
        if sources.is_empty() {
            bot.send_message(msg.chat.id, "没有订阅").await?;
            return Ok(());
        }
        let items = sources
            .iter()
            .filter_map(|s| Some((s.source_id?, s.title.clone().unwrap_or_default())))
            .collect::<Vec<_>>();
        bot.send_message(msg.chat.id, "请选择你要退订的源")
            .reply_markup(feed_item_list_keyboard(
                crate::bot::callback::Button::UnsubFeedItem,
                msg.chat.id.0,
                &items,
            ))
            .await?;
        return Ok(());
    }

    match state.repo.source_by_link(payload).await.map_err(to_request_error)? {
        None => {
            bot.send_message(msg.chat.id, "未订阅该RSS源").await?;
        }
        Some(source) => {
            if state.repo.unsubscribe_user(msg.chat.id.0, source.id).await.map_err(to_request_error)? {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "[{}]({}) 退订成功！",
                        source.title.as_deref().unwrap_or(""),
                        source.link.as_deref().unwrap_or("")
                    ),
                )
                .parse_mode(ParseMode::Markdown)
                .link_preview_options(no_preview())
                .await?;
            } else {
                bot.send_message(msg.chat.id, "退订失败").await?;
            }
        }
    }
    Ok(())
}

async fn handle_set_tag(bot: &Bot, msg: &Message, state: &BotState, payload: &str) -> ResponseResult<()> {
    let mut parts = payload.split_whitespace();
    let Some(source_id) = parts.next().and_then(|s| s.parse::<i64>().ok()) else {
        bot.send_message(msg.chat.id, "/setfeedtag [sourceID] [tag1] [tag2] 设置订阅标签（最多设置三个Tag，以空格分割）").await?;
        return Ok(());
    };
    // Go: `subscription.Tag = "#" + strings.Join(tags, " #")` — note this
    // yields the literal tag "#" when no tags are given, which we replicate.
    let tag = parts.take(3).collect::<Vec<_>>().join(" ");
    let tag = format!("#{}", tag.replace(' ', " #"));
    if state.repo.set_subscription_tag(msg.chat.id.0, source_id, &tag).await.map_err(to_request_error)? {
        bot.send_message(msg.chat.id, "订阅标签设置成功!").await?;
    } else {
        bot.send_message(msg.chat.id, "订阅标签设置失败!").await?;
    }
    Ok(())
}

async fn handle_set_interval(bot: &Bot, msg: &Message, state: &BotState, payload: &str) -> ResponseResult<()> {
    let mut parts = payload.split_whitespace();
    let Some(interval) = parts.next().and_then(|s| s.parse::<i64>().ok()).filter(|i| *i > 0) else {
        bot.send_message(msg.chat.id, "/setinterval [interval] [sourceID] 设置订阅刷新频率（可设置多个sub id，以空格分割）").await?;
        return Ok(());
    };
    let mut ok = true;
    for source_id in parts.filter_map(|s| s.parse::<i64>().ok()) {
        ok &= state.repo.set_subscription_interval(msg.chat.id.0, source_id, interval).await.map_err(to_request_error)?;
    }
    bot.send_message(msg.chat.id, if ok { "抓取频率设置成功!" } else { "抓取频率设置失败!" }).await?;
    Ok(())
}

/// Port of Go's `Set.Handle`: shows one inline button per subscribed source;
/// tapping one opens the toggle screen handled in `callbacks.rs`.
async fn handle_set(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let sources = state.repo.subscriptions_for_user(msg.chat.id.0).await.map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(msg.chat.id, "当前没有订阅").await?;
        return Ok(());
    }
    let items = sources
        .iter()
        .filter_map(|s| Some((s.source_id?, s.title.clone().unwrap_or_default())))
        .collect::<Vec<_>>();
    bot.send_message(msg.chat.id, "请选择你要设置的源")
        .reply_markup(feed_item_list_keyboard(crate::bot::callback::Button::SetFeedItem, msg.chat.id.0, &items))
        .await?;
    Ok(())
}

async fn handle_check(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let count = state.repo.mark_user_sources_due(msg.chat.id.0).await.map_err(to_request_error)?;
    if count == 0 {
        bot.send_message(msg.chat.id, "当前没有订阅").await?;
    } else {
        bot.send_message(msg.chat.id, format!("已开始检查当前订阅，共{}个源", count)).await?;
    }
    Ok(())
}

/// Port of Go's `PauseAll`/`ActiveAll`: these pause/resume the *source*
/// (`error_count`) for every source the caller is subscribed to, not a
/// per-subscriber flag — see `Core.DisableSourceUpdate`/`EnableSourceUpdate`.
#[allow(deprecated)]
async fn set_all_sources_update(bot: &Bot, msg: &Message, state: &BotState, enable: bool) -> ResponseResult<()> {
    let sources = match state.repo.subscriptions_for_user(msg.chat.id.0).await {
        Ok(sources) => sources,
        Err(_) => {
            bot.send_message(msg.chat.id, "系统错误").await?;
            return Ok(());
        }
    };
    for source in &sources {
        let Some(source_id) = source.source_id else { continue };
        let result = if enable {
            state.repo.enable_source_update(source_id).await
        } else {
            state.repo.disable_source_update(source_id).await
        };
        if result.is_err() {
            bot.send_message(msg.chat.id, if enable { "激活失败" } else { "暂停失败" }).await?;
            return Ok(());
        }
    }
    let reply = if enable { "订阅已全部开启" } else { "订阅已全部暂停" };
    bot.send_message(msg.chat.id, reply).parse_mode(ParseMode::Markdown).link_preview_options(no_preview()).await?;
    Ok(())
}

async fn handle_export(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let sources = state.repo.subscriptions_for_user(msg.chat.id.0).await.map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(msg.chat.id, "订阅列表为空").await?;
        return Ok(());
    }
    let opml_sources = sources
        .iter()
        .map(|s| OpmlSource { title: s.title.clone().unwrap_or_default(), xml_url: s.link.clone().unwrap_or_default() })
        .collect::<Vec<_>>();
    let Ok(opml_text) = export_opml(&opml_sources) else {
        bot.send_message(msg.chat.id, "导出失败").await?;
        return Ok(());
    };

    let file_name = format!("subscriptions_{}.opml", now_unix());
    let document = teloxide::types::InputFile::memory(opml_text.into_bytes()).file_name(file_name);
    if bot.send_document(msg.chat.id, document).await.is_err() {
        bot.send_message(msg.chat.id, "导出失败").await?;
    }
    Ok(())
}

#[allow(deprecated)]
async fn list_subscriptions(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let sources = state.repo.subscriptions_for_user(msg.chat.id.0).await.map_err(to_request_error)?;
    if sources.is_empty() {
        bot.send_message(msg.chat.id, "订阅列表为空").await?;
        return Ok(());
    }
    let mut text = format!("共订阅{}个源，订阅列表\n", sources.len());
    for source in sources {
        text.push_str(&format!(
            "[[{}]] [{}]({})\n",
            source.source_id.unwrap_or_default(),
            source.title.unwrap_or_default(),
            source.link.unwrap_or_default()
        ));
    }
    bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Markdown).link_preview_options(no_preview()).await?;
    Ok(())
}

/// Byte-for-byte port of the Go `/help` text (`internal/bot/handler/help.go`),
/// including its stray reference to `/check`, which has no handler in either
/// version — copied verbatim rather than "corrected".
fn help_message() -> &'static str {
    "\n\t命令：\n\t/sub 订阅源\n\t/unsub  取消订阅\n\t/list 查看当前订阅源\n\t/set 设置订阅\n\t/check 检查当前订阅\n\t/setfeedtag 设置订阅标签\n\t/setinterval 设置订阅刷新频率\n\t/activeall 开启所有订阅\n\t/pauseall 暂停所有订阅\n\t/help 帮助\n\t/import 导入 OPML 文件\n\t/export 导出 OPML 文件\n\t/unsuball 取消所有订阅\n\t详细使用方法请看：https://github.com/indes/flowerss-bot\n\t"
}

pub(crate) fn no_preview() -> LinkPreviewOptions {
    LinkPreviewOptions { is_disabled: true, url: None, prefer_small_media: false, prefer_large_media: false, show_above_text: false }
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

pub(crate) fn to_request_error(err: impl std::error::Error + Send + Sync + 'static) -> teloxide::RequestError {
    teloxide::RequestError::Io(std::sync::Arc::new(std::io::Error::other(err)))
}
