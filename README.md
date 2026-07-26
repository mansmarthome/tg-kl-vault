# flowerss-bot

A self-hostable Telegram RSS bot. This repository is a Rust rewrite of the original [`indes/flowerss-bot`](https://github.com/indes/flowerss-bot), keeping the existing SQLite database layout and Telegram command behavior as compatible as possible.

## Features

- Subscribe Telegram private chats and chats where the bot receives commands to RSS/Atom feeds.
- Periodic feed fetching, parsing, deduplication, and Telegram delivery.
- SQLite storage compatible with the original Go database schema.
- OPML import/export for bulk subscription management.
- Inline buttons for subscription settings and unsubscribe flows.
- SOCKS5 proxy support for feed fetching.
- Optional custom Telegram Bot API endpoint.
- Docker / Docker Compose deployment.
- Runtime configuration through TOML and environment variables.

## Supported commands

```text
/start                    开始使用
/sub [url]                订阅RSS源
/unsub [source_id]         退订RSS源
/list                     已订阅的RSS源
/set                      设置订阅
/check                   检查当前订阅
/setfeedtag [id] [tags]    设置rss订阅标签
/setinterval [min] [ids]   设置订阅刷新频率
/unsuball                 取消所有订阅
/activeall                开启抓取订阅更新
/pauseall                 停止抓取所有订阅更新
/import                   导入OPML文件
/export                   导出OPML
/ping                     health check
/help                     帮助
/version                  Bot 版本信息
```

`/check` immediately fetches the current chat's subscribed sources, sends newly detected items, and finishes with a summary such as `检查完成：新增0篇，67个源无更新，0个源失败`.

## Current implementation notes

Implemented:

- Telegram command and callback dispatcher, including `/check` for manual subscription refresh.
- SQLite migrations and repository methods.
- Feed fetch/parse/dedup pipeline.
- OPML import/export.
- Message rendering and Telegram sending pipeline.
- 429 retry handling with Telegram `retry_after`.
- Auto-unsubscribe on Telegram `Forbidden` errors.
- Graceful shutdown on SIGINT/SIGTERM.
- Retention pruning for old `contents` rows while keeping a dedup baseline.
- Telegraph preview publishing through the local `telegraph` crate, including HTML-to-Telegraph node conversion, `createPage`, round-robin token selection, and `FLOOD_WAIT_n` cooldown handling.

Not yet implemented / limitations:

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
git clone https://github.com/siygle/flowerss-bot.git
cd flowerss-bot
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

Every config value can be supplied through environment variables with the `FLOWERSS_` prefix. Environment variables override `config.toml` and also work when no config file is mounted.

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

List values accept comma-separated values. Bracketed forms also work, for example `FLOWERSS_ALLOWED_USERS="[123,-100]"`.

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
docker build -t flowerss-bot:latest .
```

Run:

```bash
docker run -d \
  --name flowerss-bot \
  --restart unless-stopped \
  -e FLOWERSS_BOT_TOKEN="123456:telegram-bot-token" \
  -e FLOWERSS_SQLITE_PATH="/app/data/data.db" \
  -v "$PWD/data:/app/data" \
  flowerss-bot:latest
```

Logs:

```bash
docker logs -f flowerss-bot
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
Description=flowerss-bot
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

When Telegram returns `Forbidden`, the bot automatically unsubscribes that user/chat from the affected source, matching the intended production behavior.

## License

MIT OR Apache-2.0, matching the Rust workspace metadata.
