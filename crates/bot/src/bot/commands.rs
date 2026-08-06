use teloxide::utils::command::BotCommands;

/// Bot commands registered by the Go version plus `/check`, which appears in
/// the legacy help text and forces the current chat's subscriptions due.
#[derive(Debug, Clone, PartialEq, Eq, BotCommands)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    #[command(description = "开始使用")]
    Start,
    #[command(description = "订阅RSS源")]
    Sub(String),
    #[command(description = "退订RSS源")]
    Unsub(String),
    #[command(description = "已订阅的RSS源")]
    List,
    #[command(description = "设置订阅")]
    Set,
    #[command(description = "设置")]
    Settings,
    #[command(description = "检查当前订阅")]
    Check,
    #[command(description = "设置rss订阅标签")]
    Setfeedtag(String),
    #[command(description = "取消所有订阅")]
    Unsuball,
    #[command(description = "开启抓取订阅更新")]
    Activeall,
    #[command(description = "停止抓取所有订阅更新")]
    Pauseall,
    #[command(description = "")]
    Ping,
    #[command(description = "帮助")]
    Help,
    #[command(description = "Bot 版本信息")]
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
}

/// The 14 commands the Go version shipped, frozen as a Go-parity golden. New
/// commands (bookmarks) are appended to the `Command` enum, never inserted into
/// this list; the test below pins that the derived menu *begins* with exactly
/// these, in this order.
pub const COMMANDS: &[(&str, &str)] = &[
    ("start", "开始使用"),
    ("sub", "订阅RSS源"),
    ("unsub", "退订RSS源"),
    ("list", "已订阅的RSS源"),
    ("set", "设置订阅"),
    ("settings", "设置"),
    ("check", "检查当前订阅"),
    ("setfeedtag", "设置rss订阅标签"),
    ("unsuball", "取消所有订阅"),
    ("activeall", "开启抓取订阅更新"),
    ("pauseall", "停止抓取所有订阅更新"),
    ("ping", ""),
    ("help", "帮助"),
    ("version", "Bot 版本信息"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The derived command list must begin with exactly the frozen 14 Go-parity
    /// commands, in order (names and descriptions). Anything new lands after.
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
