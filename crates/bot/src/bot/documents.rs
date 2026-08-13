use std::sync::Arc;

use futures::StreamExt;
use teloxide::{net::Download, prelude::*, types::ParseMode};

use crate::{
    bot::{runtime::BotState, subscribe::create_source},
    opml::{import_opml, OpmlSource},
};

/// Port of Go's `OnDocument.Handle`: any message with a `.opml`-suffixed
/// document is treated as a bulk-subscribe request (no `/import` argument
/// needed — `/import` itself only prints instructions, see `commands.rs`).
pub async fn handle_document(bot: Bot, msg: Message, state: Arc<BotState>) -> ResponseResult<()> {
    let Some(document) = msg.document() else { return Ok(()) };

    let is_opml = document.file_name.as_deref().is_some_and(|name| name.ends_with(".opml"));
    if !is_opml {
        bot.send_message(msg.chat.id, "請傳送正確的 OPML 檔案").await?;
        return Ok(());
    }

    let bytes = match download_document(&bot, document.file.id.clone()).await {
        Ok(bytes) => bytes,
        Err(_) => {
            bot.send_message(msg.chat.id, "取得檔案失敗").await?;
            return Ok(());
        }
    };

    let outlines = match import_opml(&String::from_utf8_lossy(&bytes)) {
        Ok(outlines) => outlines,
        Err(_) => {
            bot.send_message(msg.chat.id, "取得檔案失敗").await?;
            return Ok(());
        }
    };

    let user_id = msg.chat.id.0;
    let mut success = Vec::new();
    let mut failed = Vec::new();
    for outline in outlines {
        match create_source(&state.repo, &state.fetcher, &outline.xml_url).await {
            // Go treats `ErrSubscriptionExist` as a success too (see
            // `on_document.go`); our `subscribe_user` returning `Ok(false)`
            // for "already subscribed" is the same case, not a failure.
            Ok(source) => match state.repo.subscribe_user(user_id, source.id).await {
                Ok(_) => success.push(outline),
                Err(_) => failed.push(outline),
            },
            Err(_) => failed.push(outline),
        }
    }

    let text = render_import_report(&success, &failed);
    bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Html).await?;
    Ok(())
}

async fn download_document(bot: &Bot, file_id: teloxide::types::FileId) -> anyhow::Result<Vec<u8>> {
    let file = bot.get_file(file_id).await?;
    let mut bytes = Vec::new();
    let mut stream = bot.download_file_stream(&file.path);
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk?);
    }
    Ok(bytes)
}

/// Byte-for-byte port of the report built in `OnDocument.Handle`.
fn render_import_report(success: &[OpmlSource], failed: &[OpmlSource]) -> String {
    let mut out = format!("<b>匯入成功：{}，匯入失敗：{}</b>\n", success.len(), failed.len());
    if !success.is_empty() {
        out.push_str("<b>以下訂閱源匯入成功:</b>\n");
        for (i, outline) in success.iter().enumerate() {
            push_outline_line(&mut out, i + 1, outline);
        }
        out.push('\n');
    }
    if !failed.is_empty() {
        out.push_str("<b>以下訂閱源匯入失敗:</b>\n");
        for (i, outline) in failed.iter().enumerate() {
            push_outline_line(&mut out, i + 1, outline);
        }
    }
    out
}

fn push_outline_line(out: &mut String, index: usize, outline: &OpmlSource) {
    if outline.title.is_empty() {
        out.push_str(&format!("[{index}] {}\n", outline.xml_url));
    } else {
        out.push_str(&format!("[{index}] <a href=\"{}\">{}</a>\n", outline.xml_url, outline.title));
    }
}
