use std::sync::Arc;

use teloxide::{prelude::*, types::ParseMode};

use crate::{bot::commands::{Command, COMMANDS}, config::Config, db::repo::Repo};

#[derive(Clone)]
pub struct BotState {
    pub repo: Repo,
    pub config: Config,
}

pub async fn run_bot(bot: Bot, config: Config, repo: Repo) -> anyhow::Result<()> {
    let commands = COMMANDS
        .iter()
        .filter(|(_, description)| !description.is_empty())
        .map(|(command, description)| teloxide::types::BotCommand::new(*command, *description))
        .collect::<Vec<_>>();
    bot.set_my_commands(commands).await?;

    let state = Arc::new(BotState { repo, config });
    Command::repl(bot, move |bot: Bot, msg: Message, cmd: Command| {
        let state = Arc::clone(&state);
        async move { handle_command(bot, msg, cmd, state).await }
    })
    .await;
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
        Command::List => list_subscriptions(&bot, &msg, &state.repo).await?,
        Command::Unsuball => {
            state.repo.unsubscribe_all_user(msg.chat.id.0).await.map_err(to_request_error)?;
            bot.send_message(msg.chat.id, "退订成功").await?;
        }
        Command::Pauseall => {
            state.repo.set_all_enabled(msg.chat.id.0, false).await.map_err(to_request_error)?;
            bot.send_message(msg.chat.id, "订阅已全部暂停").parse_mode(ParseMode::MarkdownV2).await?;
        }
        Command::Activeall => {
            state.repo.set_all_enabled(msg.chat.id.0, true).await.map_err(to_request_error)?;
            bot.send_message(msg.chat.id, "订阅已全部开启").parse_mode(ParseMode::MarkdownV2).await?;
        }
        Command::Sub(payload) => {
            handle_subscribe(&bot, &msg, &state.repo, payload.trim()).await?;
        }
        Command::Unsub(payload) => {
            handle_unsubscribe(&bot, &msg, &state.repo, payload.trim()).await?;
        }
        Command::Setfeedtag(payload) => {
            handle_set_tag(&bot, &msg, &state.repo, payload.trim()).await?;
        }
        Command::Setinterval(payload) => {
            handle_set_interval(&bot, &msg, &state.repo, payload.trim()).await?;
        }
        Command::Set => {
            bot.send_message(msg.chat.id, "请选择你要设置的源").await?;
        }
        Command::Import => {
            bot.send_message(msg.chat.id, "导入OPML文件").await?;
        }
        Command::Export => {
            bot.send_message(msg.chat.id, "导出OPML").await?;
        }
    }
    Ok(())
}

async fn handle_subscribe(bot: &Bot, msg: &Message, repo: &Repo, payload: &str) -> ResponseResult<()> {
    if payload.is_empty() {
        bot.send_message(
            msg.chat.id,
            "请在命令后带上需要订阅的RSS URL，例如：/sub https://justinpot.com/feed/",
        )
        .await?;
        return Ok(());
    }
    let source_id = match repo.source_by_link(payload).await.map_err(to_request_error)? {
        Some(source) => source.id,
        None => repo.insert_source(payload, payload).await.map_err(to_request_error)?,
    };
    if repo.subscribe_user(msg.chat.id.0, source_id).await.map_err(to_request_error)? {
        bot.send_message(msg.chat.id, format!("[[{source_id}]][{payload}]({payload}) 订阅成功"))
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
    } else {
        bot.send_message(msg.chat.id, "已订阅该源，请勿重复订阅").await?;
    }
    Ok(())
}

async fn handle_unsubscribe(bot: &Bot, msg: &Message, repo: &Repo, payload: &str) -> ResponseResult<()> {
    let Ok(source_id) = payload.parse::<i64>() else {
        bot.send_message(msg.chat.id, "未订阅该RSS源").await?;
        return Ok(());
    };
    if repo.unsubscribe_user(msg.chat.id.0, source_id).await.map_err(to_request_error)? {
        bot.send_message(msg.chat.id, "退订成功！").await?;
    } else {
        bot.send_message(msg.chat.id, "未订阅该RSS源").await?;
    }
    Ok(())
}

async fn handle_set_tag(bot: &Bot, msg: &Message, repo: &Repo, payload: &str) -> ResponseResult<()> {
    let mut parts = payload.split_whitespace();
    let Some(source_id) = parts.next().and_then(|s| s.parse::<i64>().ok()) else {
        bot.send_message(msg.chat.id, "/setfeedtag [sourceID] [tag1] [tag2] 设置订阅标签（最多设置三个Tag，以空格分割）").await?;
        return Ok(());
    };
    let tag = parts.take(3).collect::<Vec<_>>().join(" ");
    if repo.set_subscription_tag(msg.chat.id.0, source_id, &tag).await.map_err(to_request_error)? {
        bot.send_message(msg.chat.id, "订阅标签设置成功!").await?;
    } else {
        bot.send_message(msg.chat.id, "订阅标签设置失败!").await?;
    }
    Ok(())
}

async fn handle_set_interval(bot: &Bot, msg: &Message, repo: &Repo, payload: &str) -> ResponseResult<()> {
    let mut parts = payload.split_whitespace();
    let Some(interval) = parts.next().and_then(|s| s.parse::<i64>().ok()) else {
        bot.send_message(msg.chat.id, "/setinterval [interval] [sourceID] 设置订阅刷新频率（可设置多个sub id，以空格分割）").await?;
        return Ok(());
    };
    let mut ok = true;
    for source_id in parts.filter_map(|s| s.parse::<i64>().ok()) {
        ok &= repo.set_subscription_interval(msg.chat.id.0, source_id, interval).await.map_err(to_request_error)?;
    }
    bot.send_message(msg.chat.id, if ok { "抓取频率设置成功!" } else { "抓取频率设置失败!" }).await?;
    Ok(())
}

async fn list_subscriptions(bot: &Bot, msg: &Message, repo: &Repo) -> ResponseResult<()> {
    let sources = repo.subscriptions_for_user(msg.chat.id.0).await.map_err(to_request_error)?;
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
    bot.send_message(msg.chat.id, text).parse_mode(ParseMode::MarkdownV2).await?;
    Ok(())
}

fn help_message() -> String {
    COMMANDS
        .iter()
        .filter(|(_, description)| !description.is_empty())
        .map(|(command, description)| format!("/{command} - {description}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn to_request_error(err: impl std::error::Error + Send + Sync + 'static) -> teloxide::RequestError {
    teloxide::RequestError::Io(std::sync::Arc::new(std::io::Error::other(err)))
}
