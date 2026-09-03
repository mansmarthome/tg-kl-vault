//! Allow-list gate for inbound Telegram updates.
//!
//! When `config.allowed_users` is empty, the bot is unrestricted. When it is
//! non-empty, only listed user IDs may interact with the dispatcher; every
//! other update is silently dropped (or, for messages, answered with the
//! localized "not authorized" notice, matching the legacy flowerss-bot UX).
//!
//! `allowed_users` is documented in the README as the per-user Telegram ID
//! (the value of `Message::from().id()` for private chats and group
//! member IDs; we treat the allow-list as opaque `i64`s so operators can
//! mix personal IDs and `-100…` group IDs as they see fit).

use teloxide::types::{CallbackQuery, Message};

use crate::config::Config;

/// Returns `true` if `user_id` is allowed to interact with the bot.
///
/// An empty `allowed_users` list means unrestricted.
pub fn is_allowed(config: &Config, user_id: i64) -> bool {
    config.allowed_users.is_empty() || config.allowed_users.contains(&user_id)
}

/// User ID associated with a `Message` (the sender in private chats, the
/// author in groups / channels). `None` for system / channel-post style
/// messages that have no `from` field.
pub fn message_user_id(msg: &Message) -> Option<i64> {
    msg.from.as_ref().map(|user| user.id.0 as i64)
}

/// User ID that pressed an inline button. `None` only for malformed
/// callback queries, which we treat as unidentifiable and therefore drop.
pub fn callback_user_id(query: &CallbackQuery) -> Option<i64> {
    Some(query.from.id.0 as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(allowed: Vec<i64>) -> Config {
        Config { allowed_users: allowed, ..Config::default() }
    }

    #[test]
    fn empty_list_is_unrestricted() {
        let cfg = cfg_with(vec![]);
        assert!(is_allowed(&cfg, 1));
        assert!(is_allowed(&cfg, -100123));
        assert!(is_allowed(&cfg, i64::MIN));
    }

    #[test]
    fn non_empty_list_filters_by_exact_id() {
        let cfg = cfg_with(vec![42, -1001]);
        assert!(is_allowed(&cfg, 42));
        assert!(is_allowed(&cfg, -1001));
        assert!(!is_allowed(&cfg, 43));
        assert!(!is_allowed(&cfg, 0));
        assert!(!is_allowed(&cfg, 42_000));
    }
}

