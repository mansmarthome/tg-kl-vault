use teloxide::utils::command::BotCommands;

/// Bot commands registered by the Go version plus `/check`, which appears in
/// the legacy help text and forces the current chat's subscriptions due.
#[derive(Debug, Clone, PartialEq, Eq, BotCommands)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    #[command(description = "開始使用")]
    Start,
    #[command(description = "訂閱 RSS 源")]
    Sub(String),
    #[command(description = "退訂 RSS 源")]
    Unsub(String),
    #[command(description = "已訂閱的 RSS 源")]
    List,
    #[command(description = "設定訂閱")]
    Set,
    #[command(description = "設定")]
    Settings,
    #[command(description = "立刻抓取所有訂閱並推播新文章")]
    Check,
    #[command(description = "設定 RSS 訂閱標籤")]
    Setfeedtag(String),
    #[command(description = "取消所有訂閱")]
    Unsuball,
    #[command(description = "開啟抓取訂閱更新")]
    Activeall,
    #[command(description = "停止抓取所有訂閱更新")]
    Pauseall,
    #[command(description = "")]
    Ping,
    #[command(description = "幫助")]
    Help,
    #[command(description = "Bot 版本資訊")]
    Version,
    // Bookmarks — appended after the frozen Go-parity 14 (never inserted).
    #[command(description = "收藏網址")]
    Bm(String),
    #[command(description = "查看書籤")]
    Bookmarks,
    #[command(description = "搜尋書籤")]
    Bmsearch(String),
    // Hidden (empty description, like /ping): typeable but not in the menu.
    // Their real UI is the 📝/🏷/🗑 buttons; these are for power users.
    #[command(description = "")]
    Bmnote(String),
    #[command(description = "")]
    Bmtag(String),
    #[command(description = "")]
    Bmdel(String),
    // Feed health — appended after bookmarks, same frozen-prefix rule.
    #[command(description = "檢查訂閱的 feed 是否還有效")]
    Feedcheck,
}

/// The 14 command *names* the Go version shipped, frozen as a Go-parity golden:
/// new commands (bookmarks, feedcheck) are appended to the `Command` enum, never
/// inserted into this list, and the test below pins that the derived menu
/// *begins* with exactly these, in this order.
///
/// The descriptions are deliberately no longer byte-for-byte with Go: they are
/// display-only text and this bot's UI language is zh-TW throughout. Names still
/// are, because those are the wire format users type.
pub const COMMANDS: &[(&str, &str)] = &[
    ("start", "開始使用"),
    ("sub", "訂閱 RSS 源"),
    ("unsub", "退訂 RSS 源"),
    ("list", "已訂閱的 RSS 源"),
    ("set", "設定訂閱"),
    ("settings", "設定"),
    ("check", "立刻抓取所有訂閱並推播新文章"),
    ("setfeedtag", "設定 RSS 訂閱標籤"),
    ("unsuball", "取消所有訂閱"),
    ("activeall", "開啟抓取訂閱更新"),
    ("pauseall", "停止抓取所有訂閱更新"),
    ("ping", ""),
    ("help", "幫助"),
    ("version", "Bot 版本資訊"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The derived command list must begin with exactly the frozen 14 Go-parity
    /// command names, in order. Anything new lands after. Descriptions are
    /// checked too, so the enum and `COMMANDS` can never drift apart — but they
    /// are this repo's zh-TW text, not Go's.
    #[test]
    fn derived_commands_begin_with_frozen_go_parity_set() {
        let derived = Command::bot_commands()
            .into_iter()
            .map(|c| {
                (
                    c.command.trim_start_matches('/').to_string(),
                    c.description,
                )
            })
            .collect::<Vec<_>>();

        assert!(
            derived.len() >= COMMANDS.len(),
            "derived list unexpectedly shorter than the frozen set"
        );
        for (i, (name, desc)) in COMMANDS.iter().enumerate() {
            assert_eq!(derived[i].0, *name, "command name drift at index {i}");
            assert_eq!(derived[i].1, *desc, "command description drift at index {i}");
        }
    }
}
