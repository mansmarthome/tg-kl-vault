use teloxide::utils::command::BotCommands;

/// Bot commands registered by the Go version plus `/check`, which appears in
/// the legacy help text and forces the current chat's subscriptions due.
#[derive(Debug, Clone, PartialEq, Eq, BotCommands)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    #[command(description = "Start using the bot")]
    Start,
    #[command(description = "Subscribe to an RSS feed")]
    Sub(String),
    #[command(description = "Unsubscribe")]
    Unsub(String),
    #[command(description = "Show current subscriptions")]
    List,
    #[command(description = "Configure a subscription")]
    Set,
    #[command(description = "Bot settings")]
    Settings,
    #[command(description = "Check current subscriptions")]
    Check,
    #[command(description = "Set tags for a subscription")]
    Setfeedtag(String),
    #[command(description = "Remove all subscriptions")]
    Unsuball,
    #[command(description = "Enable all subscriptions")]
    Activeall,
    #[command(description = "Pause all subscriptions")]
    Pauseall,
    #[command(description = "")]
    Ping,
    #[command(description = "Help")]
    Help,
    #[command(description = "Bot version")]
    Version,
}

/// Command names (without descriptions) used to register the bot command
/// list. Descriptions are looked up per-locale from `Lang::command_descriptions`.
pub const COMMAND_NAMES: &[&str] = &[
    "start",
    "sub",
    "unsub",
    "list",
    "set",
    "settings",
    "check",
    "setfeedtag",
    "unsuball",
    "activeall",
    "pauseall",
    "ping",
    "help",
    "version",
];

/// Backwards-compat constant retained for `commands_list_matches_overview`.
/// Only the names are used by `run_bot` now; descriptions come from
/// `Lang::command_descriptions` so the bot menu follows the per-chat language.
pub const COMMANDS: &[(&str, &str)] = &[
    ("start", "Start using the bot"),
    ("sub", "Subscribe to an RSS feed"),
    ("unsub", "Unsubscribe"),
    ("list", "Show current subscriptions"),
    ("set", "Configure a subscription"),
    ("settings", "Bot settings"),
    ("check", "Check current subscriptions"),
    ("setfeedtag", "Set tags for a subscription"),
    ("unsuball", "Remove all subscriptions"),
    ("activeall", "Enable all subscriptions"),
    ("pauseall", "Pause all subscriptions"),
    ("ping", ""),
    ("help", "Help"),
    ("version", "Bot version"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::i18n::Lang;

    #[test]
    fn command_list_matches_overview() {
        let names = COMMAND_NAMES.to_vec();
        assert_eq!(
            names,
            vec![
                "start",
                "sub",
                "unsub",
                "list",
                "set",
                "settings",
                "check",
                "setfeedtag",
                "unsuball",
                "activeall",
                "pauseall",
                "ping",
                "help",
                "version",
            ]
        );
        assert!(names.contains(&"check"));
    }

    #[test]
    fn command_descriptions_cover_all_languages() {
        for lang in [Lang::En, Lang::ZhTw, Lang::Ru] {
            let descriptions = lang.command_descriptions();
            assert_eq!(descriptions.len(), COMMAND_NAMES.len());
            for (i, name) in COMMAND_NAMES.iter().enumerate() {
                assert_eq!(descriptions[i].0, *name);
            }
        }
    }
}
