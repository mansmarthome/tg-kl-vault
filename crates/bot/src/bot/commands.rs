use teloxide::utils::command::BotCommands;

/// Bot commands registered by the Go version. Do not add `/check`; README
/// mentioned it, but the Go source has no handler.
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
    #[command(description = "设置rss订阅标签")]
    Setfeedtag(String),
    #[command(description = "设置订阅刷新频率")]
    Setinterval(String),
    #[command(description = "取消所有订阅")]
    Unsuball,
    #[command(description = "开启抓取订阅更新")]
    Activeall,
    #[command(description = "停止抓取所有订阅更新")]
    Pauseall,
    #[command(description = "导入OPML文件")]
    Import,
    #[command(description = "导出OPML")]
    Export,
    #[command(description = "")]
    Ping,
    #[command(description = "帮助")]
    Help,
    #[command(description = "Bot 版本信息")]
    Version,
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("start", "开始使用"),
    ("sub", "订阅RSS源"),
    ("unsub", "退订RSS源"),
    ("list", "已订阅的RSS源"),
    ("set", "设置订阅"),
    ("setfeedtag", "设置rss订阅标签"),
    ("setinterval", "设置订阅刷新频率"),
    ("unsuball", "取消所有订阅"),
    ("activeall", "开启抓取订阅更新"),
    ("pauseall", "停止抓取所有订阅更新"),
    ("import", "导入OPML文件"),
    ("export", "导出OPML"),
    ("ping", ""),
    ("help", "帮助"),
    ("version", "Bot 版本信息"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_list_matches_overview_without_check() {
        let names = COMMANDS.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "start",
                "sub",
                "unsub",
                "list",
                "set",
                "setfeedtag",
                "setinterval",
                "unsuball",
                "activeall",
                "pauseall",
                "import",
                "export",
                "ping",
                "help",
                "version",
            ]
        );
        assert!(!names.contains(&"check"));
    }
}
