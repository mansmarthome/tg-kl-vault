# tg-kl-vault

A self-hostable Telegram RSS vault bot. This project was formerly named `flowerss-bot` and remains compatible with existing `flowerss-bot` SQLite data and deployment settings where possible.

This repository is a Rust rewrite derived from the original [`indes/flowerss-bot`](https://github.com/indes/flowerss-bot), keeping the existing SQLite database layout and Telegram command behavior as compatible as possible.

## Features

- Subscribe Telegram private chats and chats where the bot receives commands to RSS/Atom feeds.
- Periodic feed fetching, parsing, deduplication, and Telegram delivery.
- SQLite storage compatible with the original Go database schema.
- OPML import/export for bulk subscription management.
- Inline buttons for subscription settings and unsubscribe flows.
- SOCKS5 proxy support for feed fetching.
- Optional custom Telegram Bot API endpoint.
- Stock tracking (台股 TWSE/TPEx + US): per-chat watchlist, quotes with technical indicators, and a per-market close-of-day push report with optional AI commentary.
- Docker / Docker Compose deployment.
- Runtime configuration through TOML and environment variables.

## Supported commands

```text
/start                    開始使用
/sub [url]                訂閱RSS源
/unsub [source_id]         退訂RSS源
/list                     已訂閱的RSS源
/set                      設定訂閱
/settings                 設定（多層按鈕：OPML、更新頻率、語系、書籤）
/check                    立刻抓取所有訂閱並推播新文章
/feedcheck                檢查訂閱的 feed 是否還有效（只探測，不推播）
/bm [url]                 收藏網址（回覆含連結的訊息亦可）
/bookmarks                瀏覽書籤（分頁）
/bmsearch [keyword]       搜尋書籤
/setfeedtag [id] [tags]    設定rss訂閱標籤
/unsuball                 取消所有訂閱
/activeall                開啟抓取訂閱更新
/pauseall                 停止抓取所有訂閱更新
/ping                     health check
/help                     幫助
/version                  Bot 版本資訊
```

`/check` immediately fetches the current chat's subscribed sources, sends newly detected items, and finishes with a summary such as `檢查完成：新增0篇，忽略0篇過舊，67個源無更新，0個源失敗`.

`/feedcheck` is the diagnostic counterpart: it probes every subscribed feed concurrently and reports which ones are dead (HTTP error, unreachable, no longer valid RSS/Atom, empty, or abandoned), alongside the failure history the scheduler recorded. It never writes to `contents`, never sends an article, and never changes a source's paused/error state.

Items whose publish date is older than `[fetch] max_item_age_days` (default 30) are recorded in the dedup ledger but never pushed. Without that gate, a feed that changes its GUIDs — or whose ledger rows aged out via `retention_days` — republishes its entire back catalogue. Items with no date are unjudgeable and still go out; set `0` to disable the gate.

## Current implementation notes

Implemented:

- Telegram command and callback dispatcher, including `/check` for manual subscription refresh and `/settings` with nested OPML, refresh interval, and language buttons.
- SQLite migrations and repository methods.
- Feed fetch/parse/dedup pipeline.
- OPML import/export through `/settings` buttons.
- Message rendering and Telegram sending pipeline.
- 429 retry handling with Telegram `retry_after`.
- Telegram `Forbidden` send failures are logged without deleting subscriptions.
- Graceful shutdown on SIGINT/SIGTERM.
- Retention pruning for old `contents` rows while keeping a dedup baseline.
- Telegraph preview publishing through the local `telegraph` crate, including HTML-to-Telegraph node conversion, `createPage`, round-robin token selection, and `FLOOD_WAIT_n` cooldown handling.

Not yet implemented / limitations:

- Legacy Go source files have been removed from this repo; use the upstream project link above for Go implementation reference.
- Legacy `@channel` mention preloading and full admin-check middleware are not complete yet; use private chats or send commands directly in the chat where the bot is installed.
- Full production cut-over validation must still be done with your real bot token and production `data.db`.

## Self-hosted deployment

### 1. Create a Telegram bot

1. Open Telegram and talk to [`@BotFather`](https://t.me/BotFather).
2. Run `/newbot` and follow the prompts.
3. Copy the bot token.
4. For group/channel usage, invite the bot to the target group/channel and give it the permissions needed to read commands and send messages.

### 2. Prepare a server

Recommended minimum:

- Linux VPS or home server.
- Docker Engine + Docker Compose plugin, or a Rust toolchain if running without Docker.
- Persistent disk for `data.db`.
- Outbound HTTPS access to Telegram Bot API and subscribed RSS sites.

Clone the repository:

```bash
git clone https://github.com/siygle/tg-kl-vault.git
cd tg-kl-vault
```

### 3. Configure by environment variables or config.toml

The bot can run with **environment variables only**. `config.toml` is optional and mainly useful for non-container installs. Defaults are built in for every key except `bot_token`, which is required unless `--dry-run` is used.

Minimal environment-only setup:

```bash
export FLOWERSS_BOT_TOKEN="123456:telegram-bot-token"
export FLOWERSS_SQLITE_PATH="/app/data/data.db"
```

Optional `config.toml` setup:

```bash
cp config.example.toml config.toml
```

Example `config.toml`:

```toml
bot_token = "123456:telegram-bot-token"
telegraph_token = []
telegraph_account = ""
telegraph_author_name = "flowerss-bot"
telegraph_author_url = ""
socks5 = ""
update_interval = 10
user_agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/51.0.2704.103 Safari/537.36"
allowed_users = []
preview_text = 0
disable_web_page_preview = false
message_mode = "html"

[sqlite]
path = "/app/data/data.db"

[telegram]
endpoint = ""

[log]
level = "info"

[fetch]
concurrency = 8
retention_days = 90
```

Important fields:

| Key | Description |
|---|---|
| `bot_token` | Telegram bot token from BotFather. Required for normal runtime. |
| `socks5` | Optional SOCKS5 proxy, for example `127.0.0.1:1080`. Leave empty to disable. |
| `update_interval` | Default feed refresh interval in minutes. |
| `allowed_users` | Optional Telegram user/chat allow-list. Empty means everyone can use the bot. |
| `preview_text` | Preview text length. `0` keeps default behavior. |
| `disable_web_page_preview` | Disable Telegram link previews when sending messages. |
| `message_mode` | `html` or `markdown`. |
| `sqlite.path` | SQLite database path. In Docker Compose, use `/app/data/data.db`. |
| `telegram.endpoint` | Optional custom Telegram Bot API server endpoint. Empty means official Telegram API. |
| `log.level` | Tracing log level, for example `error`, `warn`, `info`, `debug`, `trace`. |
| `fetch.concurrency` | Number of feeds fetched concurrently. |
| `fetch.retention_days` | Delete old content rows after this many days while keeping recent dedup baseline rows. |
| `telegraph_token` | Telegraph access token list. Empty disables Telegraph previews. Multiple tokens are used round-robin and tokens that hit `FLOOD_WAIT_n` are temporarily skipped. |
| `telegraph_author_name` | Author name shown on created Telegraph pages. |
| `telegraph_author_url` | Optional author URL shown on created Telegraph pages. |

### 4. Telegraph preview setup

Telegraph previews are optional. Leave `telegraph_token = []` to disable them.

To enable Telegraph publishing, create one or more Telegraph accounts/tokens and put them in `config.toml`:

```toml
telegraph_token = ["token1", "token2"]
telegraph_author_name = "flowerss-bot"
telegraph_author_url = ""
```

One way to create a token is Telegraph's `createAccount` API:

```bash
curl -s https://api.telegra.ph/createAccount \
  -d short_name="flowerss-bot" \
  -d author_name="flowerss-bot"
```

Copy `result.access_token` from the response into `telegraph_token`.

Behavior details:

- Article HTML is converted to Telegraph nodes by the bundled `telegraph` crate.
- Relative `href` / `src` values are resolved against the article link when possible.
- Failed Telegraph publishing does not block Telegram delivery; the bot logs the error and sends the normal non-preview message.
- Multiple tokens are used round-robin.
- If Telegraph returns `FLOOD_WAIT_n`, that token is put on cooldown and the next available token is tried.

### 5. Environment variable overrides

Every config value can be supplied through environment variables with the `FLOWERSS_` prefix. The prefix is intentionally kept for deployment compatibility with existing flowerss-bot setups. Environment variables override `config.toml` and also work when no config file is mounted.

| Env var | Config key | Example |
|---|---|---|
| `FLOWERSS_BOT_TOKEN` | `bot_token` | `123456:telegram-bot-token` |
| `FLOWERSS_TELEGRAPH_TOKEN` | `telegraph_token` | `token1,token2` |
| `FLOWERSS_TELEGRAPH_ACCOUNT` | `telegraph_account` | `flowerss-bot` |
| `FLOWERSS_TELEGRAPH_AUTHOR_NAME` | `telegraph_author_name` | `flowerss-bot` |
| `FLOWERSS_TELEGRAPH_AUTHOR_URL` | `telegraph_author_url` | `https://example.com` |
| `FLOWERSS_SOCKS5` | `socks5` | `127.0.0.1:1080` |
| `FLOWERSS_UPDATE_INTERVAL` | `update_interval` | `10` |
| `FLOWERSS_USER_AGENT` | `user_agent` | `Mozilla/5.0 ...` |
| `FLOWERSS_ALLOWED_USERS` | `allowed_users` | `123456,-100987654321` |
| `FLOWERSS_PREVIEW_TEXT` | `preview_text` | `120` |
| `FLOWERSS_DISABLE_WEB_PAGE_PREVIEW` | `disable_web_page_preview` | `false` |
| `FLOWERSS_MESSAGE_MODE` | `message_mode` | `html` or `markdown` |
| `FLOWERSS_SQLITE_PATH` | `sqlite.path` | `/app/data/data.db` |
| `FLOWERSS_TELEGRAM_ENDPOINT` | `telegram.endpoint` | `https://api.telegram.org` |
| `FLOWERSS_LOG_LEVEL` | `log.level` | `info` |
| `FLOWERSS_FETCH_CONCURRENCY` | `fetch.concurrency` | `8` |
| `FLOWERSS_FETCH_RETENTION_DAYS` | `fetch.retention_days` | `90` |
| `FLOWERSS_BOOKMARK_AI_PROVIDER` | `bookmark.ai.provider` | `auto` / `gemini` / `heuristic` |
| `FLOWERSS_BOOKMARK_AI_API_KEY` | `bookmark.ai.api_key` | (Gemini key; `GEMINI_API_KEY` also honoured) |
| `FLOWERSS_BOOKMARK_AI_MODEL` | `bookmark.ai.model` | `gemini-3.1-flash-lite` |
| `FLOWERSS_BOOKMARK_AI_ENDPOINT` | `bookmark.ai.endpoint` | `https://generativelanguage.googleapis.com` |
| `FLOWERSS_BOOKMARK_AI_DAILY_QUOTA` | `bookmark.ai.daily_quota` | `200` |
| `FLOWERSS_BOOKMARK_AI_MAX_RPM` | `bookmark.ai.max_rpm` | `10` |
| `FLOWERSS_BOOKMARK_AI_MAX_TAGS` | `bookmark.ai.max_tags` | `3` |
| `FLOWERSS_BOOKMARK_AI_PAGE_SIZE` | `bookmark.ai.page_size` | `5` |
| `FLOWERSS_BOOKMARK_AI_MCP_ENDPOINT` | `bookmark.ai.mcp.endpoint` | `https://pi-mcp.example.com/mcp` |
| `FLOWERSS_BOOKMARK_AI_MCP_TOKEN` | `bookmark.ai.mcp.token` | (bearer token) |
| `FLOWERSS_BOOKMARK_AI_MCP_CF_ACCESS_CLIENT_ID` | `bookmark.ai.mcp.cf_access_client_id` | (Cloudflare Access) |
| `FLOWERSS_BOOKMARK_AI_MCP_CF_ACCESS_CLIENT_SECRET` | `bookmark.ai.mcp.cf_access_client_secret` | (Cloudflare Access) |
| `FLOWERSS_BOOKMARK_AI_MCP_TIMEOUT_SECONDS` | `bookmark.ai.mcp.timeout_seconds` | `240` |
| `FLOWERSS_BOOKMARK_AI_MCP_POLL_INTERVAL_MS` | `bookmark.ai.mcp.poll_interval_ms` | `1500` |
| `FLOWERSS_STOCK_ENABLED` | `stock.enabled` | `true` |
| `FLOWERSS_STOCK_POLL_SECONDS` | `stock.poll_seconds` | `60` |
| `FLOWERSS_STOCK_HISTORY_DAYS` | `stock.history_days` | `180` |
| `FLOWERSS_STOCK_CACHE_TTL_SECONDS` | `stock.cache_ttl_seconds` | `60` |
| `FLOWERSS_STOCK_MAX_SYMBOLS_PER_CHAT` | `stock.max_symbols_per_chat` | `50` |
| `FLOWERSS_STOCK_MAX_SYMBOLS_GLOBAL` | `stock.max_symbols_global` | `300` |
| `FLOWERSS_STOCK_WATCHLIST_PAGE_SIZE` | `stock.watchlist_page_size` | `8` |
| `FLOWERSS_STOCK_DEFAULT_DELAY_MINUTES_TW` | `stock.default_delay_minutes_tw` | `60` |
| `FLOWERSS_STOCK_DEFAULT_DELAY_MINUTES_US` | `stock.default_delay_minutes_us` | `90` |
| `FLOWERSS_STOCK_LATE_THRESHOLD_MINUTES` | `stock.late_threshold_minutes` | `30` |
| `FLOWERSS_STOCK_TW_PROBE_SYMBOL` | `stock.tw_probe_symbol` | `2330.TW` |
| `FLOWERSS_STOCK_US_PROBE_SYMBOL` | `stock.us_probe_symbol` | `AAPL` |
| `FLOWERSS_STOCK_YAHOO_ENDPOINT` | `stock.yahoo_endpoint` | `https://query1.finance.yahoo.com` |
| `FLOWERSS_STOCK_TWSE_ENDPOINT` | `stock.twse_endpoint` | `https://openapi.twse.com.tw/v1` |
| `FLOWERSS_STOCK_TPEX_ENDPOINT` | `stock.tpex_endpoint` | `https://www.tpex.org.tw/openapi/v1` |
| `FLOWERSS_STOCK_AI_COMMENTARY` | `stock.ai_commentary` | `false` |
| `FLOWERSS_STOCK_AI_DAILY_QUOTA` | `stock.ai_daily_quota` | `20` |
| `FLOWERSS_STOCK_AI_MAX_SYMBOLS` | `stock.ai_max_symbols` | `3` |
| `FLOWERSS_STOCK_AI_REPORT_TIMEOUT_SECONDS` | `stock.ai_report_timeout_seconds` | `90` |

List values accept comma-separated values. Bracketed forms also work, for example `FLOWERSS_ALLOWED_USERS="[123,-100]"`.

### 5b. Remote database (Turso embedded replica)

The bot uses [libSQL](https://github.com/tursodatabase/libsql) for storage. By default it opens a plain local SQLite file at `FLOWERSS_SQLITE_PATH` — no configuration needed, and existing `data.db` files open as-is.

Set both of the following to run instead as a **Turso embedded replica**: writes are sent to the remote Turso primary (durable immediately), reads are served from the local file, and a background task syncs every 60s. This gives you cloud persistence/backup and multi-location access without changing any application behaviour.

| Env var | Example |
|---|---|
| `TURSO_DATABASE_URL` | `libsql://your-db.turso.io` |
| `TURSO_AUTH_TOKEN` | (database auth token from `turso db tokens create`) |

`FLOWERSS_SQLITE_PATH` is still used as the local replica file. Leave `TURSO_DATABASE_URL` unset for local-only mode. Schema migrations run automatically in both modes, and a database previously managed by the old sqlx-based build is detected (via its `_sqlx_migrations` table) so migrations are never re-applied.

**First enablement note.** An embedded replica needs a sidecar `{path}-info` metadata file next to the local db. A plain pre-Turso `data.db` has no such file, and libsql refuses that state (`db file exists but metadata file does not`). On startup the bot detects this orphan state, renames the local file to `{path}.pre-turso.<ts>.bak` (plus `-wal`/`-shm` if present), then bootstraps a fresh replica from the remote primary. If the remote is still empty and you need the old local data, import the `.bak` into Turso first (e.g. `sqlite3 data.db.pre-turso.*.bak .dump | turso db shell <name>`), then restart.

### Bookmarks + AI auto-tagging

Each chat has a bookmark library. A 🔖 button appears under every pushed item (toggle it in `/settings → 🔖 Bookmarks`), and `/bm <url>` bookmarks any URL. Saving replies immediately; a background worker then auto-tags the bookmark and edits the message. See `docs/usage.md` for the command list.

- **Tagger.** `provider = "auto"` (the default) uses Google Gemini's free tier when an `api_key` is present, otherwise a local keyword heuristic that needs no key and works offline. `provider = "mcp"` instead drives a remote agent (see below). Tags come from a fixed English-slug category table — the AI can only pick from it.
- **MCP remote agent.** Point `[bookmark.ai.mcp]` at a [pi-mcp-bridge](https://github.com/siygle/pi-mcp-bridge) endpoint (a stateless Streamable-HTTP MCP server) to have your own local agent do the AI work. With `provider = "mcp"` it does the auto-tagging; and whenever the bridge is configured, pushed items gain an on-demand **📝 summary button** — tapping it asks the agent to fetch the article and summarize it, replying with the result. Calls use the async job tools (`pi_run_async` + `pi_result` polling) so long agent turns don't hit a proxy timeout. The `token` is a shell-grade credential (the agent can run tools on your machine) — keep it behind a tunnel + ACL. If the bridge is unreachable, tagging falls back to the heuristic.
- **Quota.** `daily_quota` and `max_rpm` are **conservative guards, not official figures**: Google no longer publishes per-model free-tier numbers, so check your real quota in [AI Studio](https://aistudio.google.com/) and adjust. The authoritative protection is the client's 429 latch (which cools down, and latches to the next day when the body signals a daily cap); a bad API key disables Gemini for the process after one logged error. The offline heuristic is the final, never-failing fallback.
- **Search** uses SQLite `LIKE`: it is **ASCII case-insensitive only** (CJK text is case-sensitive), and `%`/`_` are treated as literals.
- **URL normalization** strips common tracking params (`utm_*`, `fbclid`, …) but keeps `ref` and `si`, and does **not** strip `www.` or a trailing slash. Consequence: `www.x.com/a` and `x.com/a` are two separate bookmarks.
- **Authorization.** Bookmark commands respect `allowed_users` when it is non-empty. In groups any member can read/add; deleting or retagging requires the creator or a chat admin.

### 6. Run with Docker Compose

The included `docker-compose.yml` is environment-first and only mounts `./data/` as `/app/data/`.

Create the data directory and `.env` file:

```bash
mkdir -p data
cat > .env <<'EOF'
FLOWERSS_BOT_TOKEN=123456:telegram-bot-token
# Optional overrides:
# FLOWERSS_ALLOWED_USERS=123456,-100987654321
# FLOWERSS_TELEGRAPH_TOKEN=token1,token2
# FLOWERSS_LOG_LEVEL=info
EOF
```

Start the bot:

```bash
docker compose up -d --build
```

View logs:

```bash
docker compose logs -f flowerss
```

Stop:

```bash
docker compose down
```

Upgrade:

```bash
git pull
docker compose up -d --build
```

### 7. Run with Docker directly

Build:

```bash
docker build -t tg-kl-vault:latest .
```

Run:

```bash
docker run -d \
  --name tg-kl-vault \
  --restart unless-stopped \
  -e FLOWERSS_BOT_TOKEN="123456:telegram-bot-token" \
  -e FLOWERSS_SQLITE_PATH="/app/data/data.db" \
  -v "$PWD/data:/app/data" \
  tg-kl-vault:latest
```

Logs:

```bash
docker logs -f tg-kl-vault
```

### 8. Run from source

Install Rust, then:

```bash
cargo build --release -p flowerss-bot
FLOWERSS_BOT_TOKEN="123456:telegram-bot-token" \
FLOWERSS_SQLITE_PATH="./data.db" \
./target/release/flowerss-bot

# Or with an optional config file:
./target/release/flowerss-bot -c config.toml
```

Dry-run mode:

```bash
cargo run -p flowerss-bot -- --dry-run
# Or with an optional config file:
cargo run -p flowerss-bot -- --dry-run -c config.toml
```

`--dry-run` loads config, opens SQLite, runs migrations, and exercises scheduler/fetch/dedup logic without Telegram sends.

### 9. systemd service example

If you run the release binary directly:

```ini
[Unit]
Description=tg-kl-vault
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/flowerss-bot
ExecStart=/opt/flowerss-bot/flowerss-bot
Restart=always
RestartSec=5
Environment=FLOWERSS_BOT_TOKEN=123456:telegram-bot-token
Environment=FLOWERSS_SQLITE_PATH=/opt/flowerss-bot/data.db
Environment=FLOWERSS_LOG_LEVEL=info

[Install]
WantedBy=multi-user.target
```

Install and start:

```bash
sudo cp flowerss-bot.service /etc/systemd/system/flowerss-bot.service
sudo systemctl daemon-reload
sudo systemctl enable --now flowerss-bot
sudo journalctl -u flowerss-bot -f
```

### 10. Migrating from an existing Go deployment

The Rust rewrite is designed to open the original Go `data.db` directly.

Recommended migration flow:

1. Stop the old Go bot.
2. Back up the database:

   ```bash
   cp data.db data.db.bak.$(date +%Y%m%d-%H%M%S)
   ```

3. Put the DB where the Rust bot expects it:

   - Docker Compose: `./data/data.db`
   - Source/systemd: whatever path is set in `[sqlite].path`

4. Run a dry-run first:

   ```bash
   cargo run -p flowerss-bot -- --dry-run -c config.toml

   # Or env-only:
   FLOWERSS_SQLITE_PATH="./data.db" cargo run -p flowerss-bot -- --dry-run
   ```

   or inside Docker:

   ```bash
   docker compose run --rm flowerss --dry-run
   ```

5. Start the Rust bot.
6. Watch logs and test `/ping`, `/list`, `/sub`, `/export` from Telegram.

The migrations are additive and keep the legacy tables/columns. Do not delete the original `data.db` until the Rust bot has been verified in production.

## Development

Run tests:

```bash
cargo test --workspace
```

Run clippy:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Run local dry-run:

```bash
cargo run -p flowerss-bot -- --dry-run -c config.example.toml
```

## Troubleshooting

### Bot does not reply

- Confirm `bot_token` is correct.
- Check container or systemd logs.
- Make sure the bot was started with the expected config file.
- If using a custom `telegram.endpoint`, verify it is reachable.

### SQLite file is missing or empty

- In Docker, ensure `./data` exists and is mounted.
- Confirm `[sqlite].path = "/app/data/data.db"` for Docker Compose.
- Check file permissions for the user running the container/binary.

### Feed cannot be fetched

- Check outbound network connectivity.
- If your network requires a proxy, set `socks5`.
- Verify the feed URL is reachable with `curl` from the same host.

### Telegram 429 rate limit

The bot retries once after Telegram's `retry_after` value. If rate limits keep happening, lower `fetch.concurrency` or increase feed intervals.

### Telegram Forbidden errors

When Telegram returns `Forbidden`, the bot logs the send failure but keeps the subscription. This avoids accidental subscription loss during import/manual check flows; remove the chat manually if it should no longer receive updates.

## License

MIT OR Apache-2.0, matching the Rust workspace metadata.
