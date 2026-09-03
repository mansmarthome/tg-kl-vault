use std::{env, path::Path};

use figment::{
    providers::{Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};

/// Telegram connection mode (HA-style: polling default, broadcast for send-only).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TelegramMode {
    /// Long-poll `getUpdates` + inbound command/callback dispatcher + outbound
    /// sends. This is the default and matches the original `flowerss-bot`
    /// behavior.
    #[default]
    Polling,
    /// Outbound only: scheduler and workers still send messages, but no
    /// `Dispatcher` runs, so `/sub`, `/check`, inline buttons, and document
    /// imports stop working. Use this for steady-state operation behind a
    /// self-hosted Bot API when no inbound commands are needed.
    Broadcast,
    // `Webhook` is intentionally not part of v1. See `TelegramMode` docs
    // above and the polling-broadcast plan for the non-goal.
}

pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/51.0.2704.103 Safari/537.36";
pub const DEFAULT_UPDATE_INTERVAL_MINUTES: u64 = 10;
pub const ERROR_THRESHOLD: u32 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub bot_token: String,
    pub telegraph_token: Vec<String>,
    pub telegraph_account: String,
    pub telegraph_author_name: String,
    pub telegraph_author_url: String,
    pub socks5: String,
    pub update_interval: u64,
    pub user_agent: String,
    pub allowed_users: Vec<i64>,
    pub disable_web_page_preview: bool,
    pub sqlite: SqliteConfig,
    pub telegram: TelegramConfig,
    pub log: LogConfig,
    pub fetch: FetchConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            telegraph_token: Vec::new(),
            telegraph_account: String::new(),
            // Keep the Go default. This is visible only in Telegraph metadata,
            // but still should remain byte-compatible unless configured.
            telegraph_author_name: "flowerss-bot".to_owned(),
            telegraph_author_url: String::new(),
            socks5: String::new(),
            update_interval: DEFAULT_UPDATE_INTERVAL_MINUTES,
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            allowed_users: Vec::new(),
            disable_web_page_preview: false,
            sqlite: SqliteConfig::default(),
            telegram: TelegramConfig::default(),
            log: LogConfig::default(),
            fetch: FetchConfig::default(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let mut figment = Figment::from(Serialized::defaults(Self::default()));
        if let Some(path) = path {
            figment = figment.merge(Toml::file(path));
        }
        let mut cfg: Self = figment.extract()?;
        cfg.apply_env_overrides()?;
        Ok(cfg)
    }

    fn apply_env_overrides(&mut self) -> anyhow::Result<()> {
        set_string(&mut self.bot_token, "FLOWERSS_BOT_TOKEN");
        set_string_vec(&mut self.telegraph_token, "FLOWERSS_TELEGRAPH_TOKEN");
        set_string(&mut self.telegraph_account, "FLOWERSS_TELEGRAPH_ACCOUNT");
        set_string(&mut self.telegraph_author_name, "FLOWERSS_TELEGRAPH_AUTHOR_NAME");
        set_string(&mut self.telegraph_author_url, "FLOWERSS_TELEGRAPH_AUTHOR_URL");
        set_string(&mut self.socks5, "FLOWERSS_SOCKS5");
        set_parse(&mut self.update_interval, "FLOWERSS_UPDATE_INTERVAL")?;
        set_string(&mut self.user_agent, "FLOWERSS_USER_AGENT");
        set_i64_vec(&mut self.allowed_users, "FLOWERSS_ALLOWED_USERS")?;
        set_parse(&mut self.disable_web_page_preview, "FLOWERSS_DISABLE_WEB_PAGE_PREVIEW")?;
        set_string(&mut self.sqlite.path, "FLOWERSS_SQLITE_PATH");
        set_string(&mut self.telegram.endpoint, "FLOWERSS_TELEGRAM_ENDPOINT");
        set_string_mode(&mut self.telegram.mode, "FLOWERSS_TELEGRAM_MODE")?;
        set_string(&mut self.log.level, "FLOWERSS_LOG_LEVEL");
        set_parse(&mut self.fetch.concurrency, "FLOWERSS_FETCH_CONCURRENCY")?;
        set_parse(&mut self.fetch.retention_days, "FLOWERSS_FETCH_RETENTION_DAYS")?;
        Ok(())
    }
}

fn set_string(target: &mut String, key: &str) {
    if let Ok(value) = env::var(key) {
        *target = value;
    }
}

fn set_string_mode(target: &mut TelegramMode, key: &str) -> anyhow::Result<()> {
    if let Ok(value) = env::var(key) {
        *target = parse_telegram_mode(&value)
            .ok_or_else(|| anyhow::anyhow!("invalid {key}: {value:?} (expected polling or broadcast)"))?;
    }
    Ok(())
}

fn parse_telegram_mode(raw: &str) -> Option<TelegramMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "polling" => Some(TelegramMode::Polling),
        "broadcast" => Some(TelegramMode::Broadcast),
        _ => None,
    }
}

fn set_parse<T>(target: &mut T, key: &str) -> anyhow::Result<()>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    if let Ok(value) = env::var(key) {
        *target = value.parse().map_err(|err| anyhow::anyhow!("invalid {key}: {err}"))?;
    }
    Ok(())
}

fn set_string_vec(target: &mut Vec<String>, key: &str) {
    if let Ok(value) = env::var(key) {
        *target = parse_string_vec(&value);
    }
}

fn set_i64_vec(target: &mut Vec<i64>, key: &str) -> anyhow::Result<()> {
    if let Ok(value) = env::var(key) {
        *target = parse_i64_vec(&value).map_err(|err| anyhow::anyhow!("invalid {key}: {err}"))?;
    }
    Ok(())
}

fn parse_string_vec(raw: &str) -> Vec<String> {
    split_vec_tokens(raw).map(str::to_owned).collect()
}

fn parse_i64_vec(raw: &str) -> Result<Vec<i64>, std::num::ParseIntError> {
    parse_vec_tokens(raw, str::parse)
}

fn parse_vec_tokens<T, E>(raw: &str, parse: impl Fn(&str) -> Result<T, E>) -> Result<Vec<T>, E> {
    split_vec_tokens(raw).map(parse).collect()
}

fn split_vec_tokens(raw: &str) -> impl Iterator<Item = &str> {
    raw.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|part| part.trim().trim_matches('"').trim_matches('\''))
        .filter(|part| !part.is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SqliteConfig {
    pub path: String,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self { path: "./data.db".to_owned() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct TelegramConfig {
    /// Official API or self-hosted Bot API base, e.g.
    /// `http://telegram-bot-api:8081`. Empty means the official
    /// `https://api.telegram.org`.
    pub endpoint: String,
    /// `polling` (default) | `broadcast`. Unknown values fail to parse so a
    /// typo in the config does not silently fall back to polling.
    pub mode: TelegramMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LogConfig {
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        // Go sample uses "release"; Rust tracing uses a standard level.
        Self { level: "info".to_owned() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FetchConfig {
    pub concurrency: usize,
    pub retention_days: u32,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self { concurrency: 8, retention_days: 90 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn defaults_match_go_sample_and_sanctioned_deviations() {
        let cfg = Config::default();
        assert_eq!(cfg.update_interval, 10);
        assert_eq!(cfg.user_agent, DEFAULT_USER_AGENT);
        assert_eq!(ERROR_THRESHOLD, 100);
        assert_eq!(cfg.fetch.concurrency, 8);
        assert_eq!(cfg.fetch.retention_days, 90);
    }

    #[test]
    fn env_overrides_cover_all_config_keys_without_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let keys = [
            ("FLOWERSS_BOT_TOKEN", "bot-token"),
            ("FLOWERSS_TELEGRAPH_TOKEN", "token-a,token-b"),
            ("FLOWERSS_TELEGRAPH_ACCOUNT", "acct"),
            ("FLOWERSS_TELEGRAPH_AUTHOR_NAME", "author"),
            ("FLOWERSS_TELEGRAPH_AUTHOR_URL", "https://example.com/author"),
            ("FLOWERSS_SOCKS5", "127.0.0.1:1080"),
            ("FLOWERSS_UPDATE_INTERVAL", "15"),
            ("FLOWERSS_USER_AGENT", "test-agent"),
            ("FLOWERSS_ALLOWED_USERS", "42,-100"),
            ("FLOWERSS_DISABLE_WEB_PAGE_PREVIEW", "true"),
            ("FLOWERSS_SQLITE_PATH", "/tmp/flowerss.db"),
            ("FLOWERSS_TELEGRAM_ENDPOINT", "https://telegram.example"),
            ("FLOWERSS_TELEGRAM_MODE", "broadcast"),
            ("FLOWERSS_LOG_LEVEL", "debug"),
            ("FLOWERSS_FETCH_CONCURRENCY", "3"),
            ("FLOWERSS_FETCH_RETENTION_DAYS", "14"),
        ];
        for (key, value) in keys {
            std::env::set_var(key, value);
        }

        let cfg = Config::load(None).unwrap();
        assert_eq!(cfg.bot_token, "bot-token");
        assert_eq!(cfg.telegraph_token, vec!["token-a", "token-b"]);
        assert_eq!(cfg.telegraph_account, "acct");
        assert_eq!(cfg.telegraph_author_name, "author");
        assert_eq!(cfg.telegraph_author_url, "https://example.com/author");
        assert_eq!(cfg.socks5, "127.0.0.1:1080");
        assert_eq!(cfg.update_interval, 15);
        assert_eq!(cfg.user_agent, "test-agent");
        assert_eq!(cfg.allowed_users, vec![42, -100]);
        assert!(cfg.disable_web_page_preview);
        assert_eq!(cfg.sqlite.path, "/tmp/flowerss.db");
        assert_eq!(cfg.telegram.endpoint, "https://telegram.example");
        assert_eq!(cfg.telegram.mode, TelegramMode::Broadcast);
        assert_eq!(cfg.log.level, "debug");
        assert_eq!(cfg.fetch.concurrency, 3);
        assert_eq!(cfg.fetch.retention_days, 14);

        for (key, _) in keys {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn default_mode_is_polling() {
        let cfg = Config::default();
        assert_eq!(cfg.telegram.mode, TelegramMode::Polling);
        assert_eq!(TelegramMode::default(), TelegramMode::Polling);
    }

    #[test]
    fn toml_mode_parses_polling_and_broadcast() {
        let toml = r#"
[telegram]
mode = "broadcast"
"#;
        let figment = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(toml));
        let cfg: Config = figment.extract().unwrap();
        assert_eq!(cfg.telegram.mode, TelegramMode::Broadcast);

        let toml = r#"
[telegram]
mode = "polling"
"#;
        let figment = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(toml));
        let cfg: Config = figment.extract().unwrap();
        assert_eq!(cfg.telegram.mode, TelegramMode::Polling);
    }

    #[test]
    fn toml_mode_rejects_unknown_value() {
        let toml = r#"
[telegram]
mode = "webhook"
"#;
        let figment = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(toml));
        let err = figment.extract::<Config>().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("telegram") && msg.contains("webhook"),
            "expected an error mentioning telegram/webhook, got: {msg}"
        );
    }

    #[test]
    fn env_mode_overrides_toml() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FLOWERSS_TELEGRAM_MODE", "broadcast");
        let mut cfg = Config::load(None).unwrap();
        // Sanity: env-driven mode wins.
        assert_eq!(cfg.telegram.mode, TelegramMode::Broadcast);
        // Clearing the env var should fall back to default (Polling).
        std::env::remove_var("FLOWERSS_TELEGRAM_MODE");
        cfg = Config::load(None).unwrap();
        assert_eq!(cfg.telegram.mode, TelegramMode::Polling);
    }

    #[test]
    fn toml_allowed_users_parses_integers() {
        let toml = r#"
allowed_users = [42, -1001]
"#;
        let figment = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(toml));
        let cfg: Config = figment.extract().unwrap();
        assert_eq!(cfg.allowed_users, vec![42, -1001]);
    }

    #[test]
    fn env_mode_rejects_unknown_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FLOWERSS_TELEGRAM_MODE", "nope");
        let result = Config::load(None);
        std::env::remove_var("FLOWERSS_TELEGRAM_MODE");
        let err = result.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("FLOWERSS_TELEGRAM_MODE") && msg.contains("nope"),
            "expected env-var error mentioning FLOWERSS_TELEGRAM_MODE/nope, got: {msg}"
        );
    }
}
