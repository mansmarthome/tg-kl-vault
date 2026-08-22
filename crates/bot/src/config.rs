use std::{env, path::Path};

use figment::{
    providers::{Format, Serialized, Toml},
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
    pub bookmark: BookmarkConfig,
    pub stock: StockConfig,
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
            bookmark: BookmarkConfig::default(),
            stock: StockConfig::default(),
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
        set_parse(&mut self.preview_text, "FLOWERSS_PREVIEW_TEXT")?;
        set_parse(&mut self.disable_web_page_preview, "FLOWERSS_DISABLE_WEB_PAGE_PREVIEW")?;
        set_parse(&mut self.message_mode, "FLOWERSS_MESSAGE_MODE")?;
        set_string(&mut self.sqlite.path, "FLOWERSS_SQLITE_PATH");
        set_string(&mut self.telegram.endpoint, "FLOWERSS_TELEGRAM_ENDPOINT");
        set_string(&mut self.log.level, "FLOWERSS_LOG_LEVEL");
        set_parse(&mut self.fetch.concurrency, "FLOWERSS_FETCH_CONCURRENCY")?;
        set_parse(&mut self.fetch.retention_days, "FLOWERSS_FETCH_RETENTION_DAYS")?;
        set_parse(&mut self.fetch.max_item_age_days, "FLOWERSS_FETCH_MAX_ITEM_AGE_DAYS")?;
        set_parse(&mut self.bookmark.ai.provider, "FLOWERSS_BOOKMARK_AI_PROVIDER")?;
        set_string(&mut self.bookmark.ai.api_key, "FLOWERSS_BOOKMARK_AI_API_KEY");
        set_string(&mut self.bookmark.ai.model, "FLOWERSS_BOOKMARK_AI_MODEL");
        set_string(&mut self.bookmark.ai.endpoint, "FLOWERSS_BOOKMARK_AI_ENDPOINT");
        set_parse(&mut self.bookmark.ai.daily_quota, "FLOWERSS_BOOKMARK_AI_DAILY_QUOTA")?;
        set_parse(&mut self.bookmark.ai.max_rpm, "FLOWERSS_BOOKMARK_AI_MAX_RPM")?;
        set_parse(&mut self.bookmark.ai.max_tags, "FLOWERSS_BOOKMARK_AI_MAX_TAGS")?;
        set_parse(&mut self.bookmark.ai.page_size, "FLOWERSS_BOOKMARK_AI_PAGE_SIZE")?;
        set_string(&mut self.bookmark.ai.mcp.endpoint, "FLOWERSS_BOOKMARK_AI_MCP_ENDPOINT");
        set_string(&mut self.bookmark.ai.mcp.token, "FLOWERSS_BOOKMARK_AI_MCP_TOKEN");
        set_string(&mut self.bookmark.ai.mcp.cf_access_client_id, "FLOWERSS_BOOKMARK_AI_MCP_CF_ACCESS_CLIENT_ID");
        set_string(&mut self.bookmark.ai.mcp.cf_access_client_secret, "FLOWERSS_BOOKMARK_AI_MCP_CF_ACCESS_CLIENT_SECRET");
        set_parse(&mut self.bookmark.ai.mcp.timeout_seconds, "FLOWERSS_BOOKMARK_AI_MCP_TIMEOUT_SECONDS")?;
        set_parse(&mut self.bookmark.ai.mcp.poll_interval_ms, "FLOWERSS_BOOKMARK_AI_MCP_POLL_INTERVAL_MS")?;
        set_parse(&mut self.stock.enabled, "FLOWERSS_STOCK_ENABLED")?;
        set_parse(&mut self.stock.poll_seconds, "FLOWERSS_STOCK_POLL_SECONDS")?;
        set_parse(&mut self.stock.history_days, "FLOWERSS_STOCK_HISTORY_DAYS")?;
        set_parse(&mut self.stock.cache_ttl_seconds, "FLOWERSS_STOCK_CACHE_TTL_SECONDS")?;
        set_parse(&mut self.stock.max_symbols_per_chat, "FLOWERSS_STOCK_MAX_SYMBOLS_PER_CHAT")?;
        set_parse(&mut self.stock.max_symbols_global, "FLOWERSS_STOCK_MAX_SYMBOLS_GLOBAL")?;
        set_parse(&mut self.stock.watchlist_page_size, "FLOWERSS_STOCK_WATCHLIST_PAGE_SIZE")?;
        set_parse(&mut self.stock.default_delay_minutes_tw, "FLOWERSS_STOCK_DEFAULT_DELAY_MINUTES_TW")?;
        set_parse(&mut self.stock.default_delay_minutes_us, "FLOWERSS_STOCK_DEFAULT_DELAY_MINUTES_US")?;
        set_parse(&mut self.stock.late_threshold_minutes, "FLOWERSS_STOCK_LATE_THRESHOLD_MINUTES")?;
        set_string(&mut self.stock.tw_probe_symbol, "FLOWERSS_STOCK_TW_PROBE_SYMBOL");
        set_string(&mut self.stock.us_probe_symbol, "FLOWERSS_STOCK_US_PROBE_SYMBOL");
        set_string(&mut self.stock.yahoo_endpoint, "FLOWERSS_STOCK_YAHOO_ENDPOINT");
        set_string(&mut self.stock.twse_endpoint, "FLOWERSS_STOCK_TWSE_ENDPOINT");
        set_string(&mut self.stock.tpex_endpoint, "FLOWERSS_STOCK_TPEX_ENDPOINT");
        set_parse(&mut self.stock.ai_commentary, "FLOWERSS_STOCK_AI_COMMENTARY")?;
        set_parse(&mut self.stock.ai_daily_quota, "FLOWERSS_STOCK_AI_DAILY_QUOTA")?;
        set_parse(&mut self.stock.ai_max_symbols, "FLOWERSS_STOCK_AI_MAX_SYMBOLS")?;
        set_parse(&mut self.stock.ai_report_timeout_seconds, "FLOWERSS_STOCK_AI_REPORT_TIMEOUT_SECONDS")?;
        // Convenience: honour a bare GEMINI_API_KEY when the namespaced one is
        // unset, so operators can use Google's standard env var name.
        if self.bookmark.ai.api_key.is_empty() {
            set_string(&mut self.bookmark.ai.api_key, "GEMINI_API_KEY");
        }
        Ok(())
    }
}

fn set_string(target: &mut String, key: &str) {
    if let Ok(value) = env::var(key) {
        *target = value;
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageMode {
    Html,
    Markdown,
}

impl std::str::FromStr for MessageMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "html" => Ok(Self::Html),
            "markdown" => Ok(Self::Markdown),
            _ => anyhow::bail!("expected html or markdown"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SqliteConfig {
    pub path: String,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        // Default under ./data/ so it lands inside the conventional mounted
        // volume (docker-compose maps ./data:/app/data); a bare ./data.db would
        // sit on the container's ephemeral layer and vanish on redeploy.
        Self { path: "./data/data.db".to_owned() }
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
    /// Never push an item whose publish date is older than this, however "new"
    /// the dedup ledger thinks it is. Guards against a feed republishing its
    /// whole archive after a GUID change and against ledger rows aged out by
    /// `retention_days`. `0` disables the gate. See `feed::parse::is_stale_item`.
    pub max_item_age_days: u32,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self { concurrency: 8, retention_days: 90, max_item_age_days: 30 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct BookmarkConfig {
    pub ai: AiConfig,
}

/// AI auto-tagging settings, `[bookmark.ai]`.
///
/// Note: `Config` derives `Eq` and is cloned widely — no `f32` fields here
/// (temperature is hardcoded in the Gemini client). `api_key` is redacted in
/// the manual `Debug` impl below so a stray `{:?}` never logs it.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AiConfig {
    pub provider: AiProvider,
    pub api_key: String,
    pub model: String,
    pub endpoint: String,
    /// Conservative soft guard, NOT an official figure — Google no longer
    /// publishes per-model free-tier numbers. The 429 latch is authoritative.
    pub daily_quota: u32,
    pub max_rpm: u32,
    pub max_tags: u32,
    pub page_size: u32,
    pub mcp: McpConfig,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProvider::Auto,
            api_key: String::new(),
            model: "gemini-3.1-flash-lite".to_owned(),
            endpoint: "https://generativelanguage.googleapis.com".to_owned(),
            daily_quota: 200,
            max_rpm: 10,
            max_tags: 3,
            page_size: 5,
            mcp: McpConfig::default(),
        }
    }
}

impl std::fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiConfig")
            .field("provider", &self.provider)
            .field("api_key", &redacted(&self.api_key))
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .field("daily_quota", &self.daily_quota)
            .field("max_rpm", &self.max_rpm)
            .field("max_tags", &self.max_tags)
            .field("page_size", &self.page_size)
            .field("mcp", &self.mcp)
            .finish()
    }
}

fn redacted(value: &str) -> &'static str {
    if value.is_empty() {
        "<unset>"
    } else {
        "<redacted>"
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    /// Gemini when an api_key is present, otherwise the local heuristic. The
    /// default: a config set to `"gemini"` that silently falls back to
    /// heuristic would be a config that lies.
    #[default]
    Auto,
    Gemini,
    Heuristic,
    /// Drive a remote agent over MCP (pi-mcp-bridge). Selected explicitly;
    /// never chosen by `auto`. Heuristic remains the offline fallback.
    Mcp,
}

impl std::str::FromStr for AiProvider {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "gemini" => Ok(Self::Gemini),
            "heuristic" => Ok(Self::Heuristic),
            "mcp" => Ok(Self::Mcp),
            _ => anyhow::bail!("expected auto, gemini, heuristic, or mcp"),
        }
    }
}

/// `[bookmark.ai.mcp]` — connection to a Streamable-HTTP MCP bridge
/// (pi-mcp-bridge) that drives a remote agent for tagging and summaries.
///
/// `token` and `cf_access_client_secret` are shell-grade credentials, redacted
/// in the manual `Debug` impl below.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct McpConfig {
    /// Full endpoint URL, e.g. `https://pi-mcp.example.com/mcp`.
    pub endpoint: String,
    /// Bearer token (mandatory server-side).
    pub token: String,
    /// Optional Cloudflare Access service-token headers.
    pub cf_access_client_id: String,
    pub cf_access_client_secret: String,
    /// Overall per-call deadline (async job polling), seconds.
    pub timeout_seconds: u64,
    /// Poll interval for `pi_result`, milliseconds.
    pub poll_interval_ms: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            token: String::new(),
            cf_access_client_id: String::new(),
            cf_access_client_secret: String::new(),
            timeout_seconds: 240,
            poll_interval_ms: 1500,
        }
    }
}

impl McpConfig {
    /// Whether the bridge is configured enough to use.
    pub fn is_configured(&self) -> bool {
        !self.endpoint.is_empty() && !self.token.is_empty()
    }
}

impl std::fmt::Debug for McpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpConfig")
            .field("endpoint", &self.endpoint)
            .field("token", &redacted(&self.token))
            .field("cf_access_client_id", &self.cf_access_client_id)
            .field("cf_access_client_secret", &redacted(&self.cf_access_client_secret))
            .field("timeout_seconds", &self.timeout_seconds)
            .field("poll_interval_ms", &self.poll_interval_ms)
            .finish()
    }
}

/// Stock tracking, `[stock]`.
///
/// Like `AiConfig`, this is reachable from a `Config` that derives `Eq`, so it
/// holds **no `f32`/`f64`** (the Bollinger `k` factor is a hardcoded call-site
/// constant, not a field). The three endpoint overrides exist so tests can
/// point the source layer at `testutil` servers — same reason `[bookmark.ai]`
/// has one. AI credentials are **not** duplicated here: the commentary path
/// reuses `[bookmark.ai.mcp]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct StockConfig {
    pub enabled: bool,
    pub poll_seconds: u64,
    pub history_days: u16,
    pub cache_ttl_seconds: u64,
    pub max_symbols_per_chat: u32,
    pub max_symbols_global: u32,
    pub watchlist_page_size: u32,
    /// Minutes after the close to push when a chat has set no explicit time.
    pub default_delay_minutes_tw: u64,
    pub default_delay_minutes_us: u64,
    /// A report fired more than this many minutes past its target is flagged
    /// "delayed" in the render.
    pub late_threshold_minutes: u64,
    /// Unconditionally fetched each pass so market state never depends on a
    /// (possibly empty) user watchlist.
    pub tw_probe_symbol: String,
    pub us_probe_symbol: String,
    pub yahoo_endpoint: String,
    pub twse_endpoint: String,
    pub tpex_endpoint: String,
    /// AI commentary is **off by default**; the numbers always render without it.
    pub ai_commentary: bool,
    /// Daily MCP-call budget, metered separately from the bookmark quota even
    /// though both hit the same bridge — a chatty watchlist must not starve
    /// bookmark tagging.
    pub ai_daily_quota: u32,
    /// How many symbols go into a single batch prompt (not how many calls).
    pub ai_max_symbols: u32,
    /// Shorter than `McpConfig::timeout_seconds` (240) so one slow agent turn
    /// can't stall the 60s worker tick.
    pub ai_report_timeout_seconds: u64,
}

impl Default for StockConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_seconds: 60,
            history_days: 180,
            cache_ttl_seconds: 60,
            max_symbols_per_chat: 50,
            max_symbols_global: 300,
            watchlist_page_size: 8,
            default_delay_minutes_tw: 60,
            default_delay_minutes_us: 90,
            late_threshold_minutes: 30,
            tw_probe_symbol: "2330.TW".to_owned(),
            us_probe_symbol: "AAPL".to_owned(),
            yahoo_endpoint: "https://query1.finance.yahoo.com".to_owned(),
            twse_endpoint: "https://openapi.twse.com.tw/v1".to_owned(),
            tpex_endpoint: "https://www.tpex.org.tw/openapi/v1".to_owned(),
            ai_commentary: false,
            ai_daily_quota: 20,
            ai_max_symbols: 3,
            ai_report_timeout_seconds: 90,
        }
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
        assert_eq!(cfg.preview_text, 0);
        assert_eq!(cfg.message_mode, MessageMode::Html);
        assert_eq!(cfg.user_agent, DEFAULT_USER_AGENT);
        assert_eq!(ERROR_THRESHOLD, 100);
        assert_eq!(cfg.fetch.concurrency, 8);
        assert_eq!(cfg.fetch.retention_days, 90);
        assert_eq!(cfg.fetch.max_item_age_days, 30);
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
            ("FLOWERSS_PREVIEW_TEXT", "120"),
            ("FLOWERSS_DISABLE_WEB_PAGE_PREVIEW", "true"),
            ("FLOWERSS_MESSAGE_MODE", "markdown"),
            ("FLOWERSS_SQLITE_PATH", "/tmp/flowerss.db"),
            ("FLOWERSS_TELEGRAM_ENDPOINT", "https://telegram.example"),
            ("FLOWERSS_LOG_LEVEL", "debug"),
            ("FLOWERSS_FETCH_CONCURRENCY", "3"),
            ("FLOWERSS_FETCH_RETENTION_DAYS", "14"),
            ("FLOWERSS_FETCH_MAX_ITEM_AGE_DAYS", "7"),
            ("FLOWERSS_BOOKMARK_AI_PROVIDER", "gemini"),
            ("FLOWERSS_BOOKMARK_AI_API_KEY", "secret-key"),
            ("FLOWERSS_BOOKMARK_AI_MODEL", "gemini-x"),
            ("FLOWERSS_BOOKMARK_AI_ENDPOINT", "https://gen.example"),
            ("FLOWERSS_BOOKMARK_AI_DAILY_QUOTA", "42"),
            ("FLOWERSS_BOOKMARK_AI_MAX_RPM", "7"),
            ("FLOWERSS_BOOKMARK_AI_MAX_TAGS", "2"),
            ("FLOWERSS_BOOKMARK_AI_PAGE_SIZE", "9"),
            ("FLOWERSS_BOOKMARK_AI_MCP_ENDPOINT", "https://pi.example/mcp"),
            ("FLOWERSS_BOOKMARK_AI_MCP_TOKEN", "mcp-secret"),
            ("FLOWERSS_BOOKMARK_AI_MCP_CF_ACCESS_CLIENT_ID", "cf-id"),
            ("FLOWERSS_BOOKMARK_AI_MCP_CF_ACCESS_CLIENT_SECRET", "cf-secret"),
            ("FLOWERSS_BOOKMARK_AI_MCP_TIMEOUT_SECONDS", "120"),
            ("FLOWERSS_BOOKMARK_AI_MCP_POLL_INTERVAL_MS", "800"),
            ("FLOWERSS_STOCK_ENABLED", "false"),
            ("FLOWERSS_STOCK_POLL_SECONDS", "45"),
            ("FLOWERSS_STOCK_HISTORY_DAYS", "200"),
            ("FLOWERSS_STOCK_CACHE_TTL_SECONDS", "30"),
            ("FLOWERSS_STOCK_MAX_SYMBOLS_PER_CHAT", "25"),
            ("FLOWERSS_STOCK_MAX_SYMBOLS_GLOBAL", "150"),
            ("FLOWERSS_STOCK_WATCHLIST_PAGE_SIZE", "6"),
            ("FLOWERSS_STOCK_DEFAULT_DELAY_MINUTES_TW", "45"),
            ("FLOWERSS_STOCK_DEFAULT_DELAY_MINUTES_US", "75"),
            ("FLOWERSS_STOCK_LATE_THRESHOLD_MINUTES", "20"),
            ("FLOWERSS_STOCK_TW_PROBE_SYMBOL", "2317.TW"),
            ("FLOWERSS_STOCK_US_PROBE_SYMBOL", "MSFT"),
            ("FLOWERSS_STOCK_YAHOO_ENDPOINT", "https://yahoo.example"),
            ("FLOWERSS_STOCK_TWSE_ENDPOINT", "https://twse.example"),
            ("FLOWERSS_STOCK_TPEX_ENDPOINT", "https://tpex.example"),
            ("FLOWERSS_STOCK_AI_COMMENTARY", "true"),
            ("FLOWERSS_STOCK_AI_DAILY_QUOTA", "12"),
            ("FLOWERSS_STOCK_AI_MAX_SYMBOLS", "5"),
            ("FLOWERSS_STOCK_AI_REPORT_TIMEOUT_SECONDS", "60"),
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
        assert_eq!(cfg.preview_text, 120);
        assert!(cfg.disable_web_page_preview);
        assert_eq!(cfg.message_mode, MessageMode::Markdown);
        assert_eq!(cfg.sqlite.path, "/tmp/flowerss.db");
        assert_eq!(cfg.telegram.endpoint, "https://telegram.example");
        assert_eq!(cfg.log.level, "debug");
        assert_eq!(cfg.fetch.concurrency, 3);
        assert_eq!(cfg.fetch.retention_days, 14);
        assert_eq!(cfg.fetch.max_item_age_days, 7);
        assert_eq!(cfg.bookmark.ai.provider, AiProvider::Gemini);
        assert_eq!(cfg.bookmark.ai.api_key, "secret-key");
        assert_eq!(cfg.bookmark.ai.model, "gemini-x");
        assert_eq!(cfg.bookmark.ai.endpoint, "https://gen.example");
        assert_eq!(cfg.bookmark.ai.daily_quota, 42);
        assert_eq!(cfg.bookmark.ai.max_rpm, 7);
        assert_eq!(cfg.bookmark.ai.max_tags, 2);
        assert_eq!(cfg.bookmark.ai.page_size, 9);
        assert_eq!(cfg.bookmark.ai.mcp.endpoint, "https://pi.example/mcp");
        assert_eq!(cfg.bookmark.ai.mcp.token, "mcp-secret");
        assert_eq!(cfg.bookmark.ai.mcp.cf_access_client_id, "cf-id");
        assert_eq!(cfg.bookmark.ai.mcp.cf_access_client_secret, "cf-secret");
        assert_eq!(cfg.bookmark.ai.mcp.timeout_seconds, 120);
        assert_eq!(cfg.bookmark.ai.mcp.poll_interval_ms, 800);
        assert!(cfg.bookmark.ai.mcp.is_configured());
        assert!(!cfg.stock.enabled);
        assert_eq!(cfg.stock.poll_seconds, 45);
        assert_eq!(cfg.stock.history_days, 200);
        assert_eq!(cfg.stock.cache_ttl_seconds, 30);
        assert_eq!(cfg.stock.max_symbols_per_chat, 25);
        assert_eq!(cfg.stock.max_symbols_global, 150);
        assert_eq!(cfg.stock.watchlist_page_size, 6);
        assert_eq!(cfg.stock.default_delay_minutes_tw, 45);
        assert_eq!(cfg.stock.default_delay_minutes_us, 75);
        assert_eq!(cfg.stock.late_threshold_minutes, 20);
        assert_eq!(cfg.stock.tw_probe_symbol, "2317.TW");
        assert_eq!(cfg.stock.us_probe_symbol, "MSFT");
        assert_eq!(cfg.stock.yahoo_endpoint, "https://yahoo.example");
        assert_eq!(cfg.stock.twse_endpoint, "https://twse.example");
        assert_eq!(cfg.stock.tpex_endpoint, "https://tpex.example");
        assert!(cfg.stock.ai_commentary);
        assert_eq!(cfg.stock.ai_daily_quota, 12);
        assert_eq!(cfg.stock.ai_max_symbols, 5);
        assert_eq!(cfg.stock.ai_report_timeout_seconds, 60);

        for (key, _) in keys {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn bare_gemini_api_key_is_a_fallback_when_namespaced_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("GEMINI_API_KEY", "bare-key");
        let cfg = Config::load(None).unwrap();
        assert_eq!(cfg.bookmark.ai.api_key, "bare-key");

        // Namespaced key wins over the bare fallback.
        std::env::set_var("FLOWERSS_BOOKMARK_AI_API_KEY", "ns-key");
        let cfg = Config::load(None).unwrap();
        assert_eq!(cfg.bookmark.ai.api_key, "ns-key");

        std::env::remove_var("GEMINI_API_KEY");
        std::env::remove_var("FLOWERSS_BOOKMARK_AI_API_KEY");
    }

    #[test]
    fn debug_config_never_prints_api_key() {
        let cfg = AiConfig {
            api_key: "super-secret".to_owned(),
            ..AiConfig::default()
        };
        let printed = format!("{cfg:?}");
        assert!(!printed.contains("super-secret"));
        assert!(printed.contains("<redacted>"));
    }

    #[test]
    fn debug_config_never_prints_mcp_secrets() {
        let cfg = McpConfig {
            token: "tok-secret".to_owned(),
            cf_access_client_secret: "cf-secret".to_owned(),
            cf_access_client_id: "cf-id-visible".to_owned(),
            ..McpConfig::default()
        };
        let printed = format!("{cfg:?}");
        assert!(!printed.contains("tok-secret"));
        assert!(!printed.contains("cf-secret"));
        // The (non-secret) client id is fine to show.
        assert!(printed.contains("cf-id-visible"));
    }
}
