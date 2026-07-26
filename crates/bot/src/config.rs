use std::path::Path;

use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/51.0.2704.103 Safari/537.36";
pub const DEFAULT_UPDATE_INTERVAL_MINUTES: u64 = 10;
pub const ERROR_THRESHOLD: u32 = 100;
pub const DEFAULT_PREVIEW_TEXT: u32 = 0;

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
    pub preview_text: u32,
    pub disable_web_page_preview: bool,
    pub message_mode: MessageMode,
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
            preview_text: DEFAULT_PREVIEW_TEXT,
            disable_web_page_preview: false,
            message_mode: MessageMode::Html,
            sqlite: SqliteConfig::default(),
            telegram: TelegramConfig::default(),
            log: LogConfig::default(),
            fetch: FetchConfig::default(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self, Box<figment::Error>> {
        let mut figment = Figment::from(Serialized::defaults(Self::default()));
        if let Some(path) = path {
            figment = figment.merge(Toml::file(path));
        }
        // FLOWERSS_BOT_TOKEN -> bot_token, FLOWERSS_SQLITE_PATH -> sqlite.path.
        figment.merge(Env::prefixed("FLOWERSS_").split("_")).extract().map_err(Box::new)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageMode {
    Html,
    Markdown,
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
    pub endpoint: String,
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

    #[test]
    fn defaults_match_go_sample_and_sanctioned_deviations() {
        let cfg = Config::default();
        assert_eq!(cfg.update_interval, 10);
        assert_eq!(cfg.preview_text, 0);
        assert_eq!(cfg.message_mode, MessageMode::Html);
        assert_eq!(cfg.user_agent, DEFAULT_USER_AGENT);
        assert_eq!(ERROR_THRESHOLD, 100);
        assert_eq!(cfg.fetch.concurrency, 8);
        assert_eq!(cfg.fetch.retention_days, 90);
    }
}
