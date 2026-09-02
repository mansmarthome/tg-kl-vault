# tg-kl-vault

Self-hostable **Telegram RSS bot** written in Rust. Subscribe private chats, groups, or channels to RSS/Atom feeds and push new items through the **Telegram Bot API** (official or self-hosted).

This fork focuses on reliable feed → Telegram delivery (including **channel** posting and a **broadcast-only** connection mode). It is **not** a bookmark/AI/trading toolkit.

## Lineage

| Project | Role |
|--------|------|
| [indes/flowerss-bot](https://github.com/indes/flowerss-bot) | Original Go bot and SQLite schema |
| [siygle/tg-kl-vault](https://github.com/siygle/tg-kl-vault) | Rust rewrite (this tree branched before bookmarks / AI tagging / stocks) |
| **This repository** | Channel-oriented fork: richer HTML descriptions, localization, Bot API server + broadcast mode |

Where practical, the SQLite layout and core command behavior stay compatible with classic `flowerss-bot` deployments.

Compared to [RSS-to-Telegram-Bot](https://github.com/Rongronggg9/RSS-to-Telegram-Bot) (MTProto-oriented), this project targets the **HTTP Bot API**, including a **self-hosted Bot API server**, so you can run outbound-only delivery without long-polling when you do not need interactive commands.

## Features

- Subscribe Telegram chats (and channels where the bot can post) to RSS/Atom feeds
- Periodic fetch, parse, deduplication, and Telegram delivery
- SQLite storage compatible with the original Go `flowerss-bot` schema (for this feature set)
- OPML import/export
- Inline controls for subscription settings and unsubscribe (when using polling mode)
- SOCKS5 proxy for feed fetching
- Custom Telegram Bot API endpoint (self-hosted server)
- **Connection modes:** `polling` (commands + callbacks) and `broadcast` (outbound sends only, no `getUpdates`)
- HTML-oriented message rendering suited to full item descriptions
- UI localization (e.g. English default, Traditional Chinese, Russian — see `/settings`)
- Docker / Compose and TOML or environment configuration

## Supported commands

(Exact labels depend on the chat language in `/settings`.)

```
/sub Subscribe to an RSS feed
/unsub Unsubscribe
/list Show subscriptions
/set Feed settings
/settings Bot settings
/check Check current subscriptions
/activeall Enable all subscriptions
/pauseall Pause all subscriptions
/unsuball Remove all subscriptions
/help Help
/version Bot version
```

## Telegram connection modes

| Mode | Default | `getUpdates` | Outbound posts | Commands / buttons |
|------|---------|--------------|----------------|--------------------|
| `polling` | yes | yes | yes | yes |
| `broadcast` | no | **no** | yes | **no** |

**Typical workflow for a news channel**

1. `mode = "polling"` — subscribe, set intervals/tags, verify a push
2. `mode = "broadcast"` — restart; scheduler keeps posting without long-polling
3. Switch back to `polling` only when you need to manage subscriptions again

Configure with `telegram.mode` or `FLOWERSS_TELEGRAM_MODE`. On `polling` startup the bot best-effort calls `deleteWebhook` so an old webhook does not block long-polling.

## Self-hosted deployment

### 1. Create a bot

1. Talk to [@BotFather](https://t.me/BotFather) → `/newbot`
2. Copy the token
3. For a **channel**, add the bot as an admin with permission to post messages

### 2. Clone and configure

```bash
git clone https://github.com/mansmarthome/tg-kl-vault.git
cd tg-kl-vault
```

Minimal env:

```bash
export FLOWERSS_BOT_TOKEN="123456:telegram-bot-token"
export FLOWERSS_SQLITE_PATH="/app/data/data.db"
# optional self-hosted Bot API:
# export FLOWERSS_TELEGRAM_ENDPOINT="http://127.0.0.1:8081"
# export FLOWERSS_TELEGRAM_MODE="broadcast"
```

Optional file:

```bash
cp config.example.toml config.toml
```

Example excerpt:

```toml
bot_token = "123456:telegram-bot-token"
update_interval = 10
allowed_users = []
disable_web_page_preview = false

[sqlite]
path = "/app/data/data.db"

[telegram]
endpoint = ""          # e.g. http://127.0.0.1:8081 for a local Bot API server
mode = "polling"       # polling | broadcast

[log]
level = "info"

[fetch]
concurrency = 8
retention_days = 90
```

| Key | Description |
|-----|-------------|
| `bot_token` | BotFather token (required unless `--dry-run`) |
| `socks5` | Optional proxy for feed fetches |
| `update_interval` | Default refresh interval (minutes) |
| `allowed_users` | Optional allow-list; empty = unrestricted |
| `sqlite.path` | Database path |
| `telegram.endpoint` | Custom Bot API base URL; empty = official API |
| `telegram.mode` | `polling` or `broadcast` |
| `telegraph_token` | Optional Telegraph tokens for Instant View–style pages |

Environment variables use the `FLOWERSS_` prefix (compatibility with flowerss-bot deploys) and override the TOML file. See `config.example.toml` for the full list (`FLOWERSS_TELEGRAM_MODE`, `FLOWERSS_TELEGRAM_ENDPOINT`, etc.).

### 3. Docker Compose

```bash
mkdir -p data
# set FLOWERSS_BOT_TOKEN in .env
docker compose up -d --build
docker compose logs -f
```

### 4. From source

```bash
cargo build --release -p flowerss-bot
FLOWERSS_BOT_TOKEN="…" FLOWERSS_SQLITE_PATH="./data.db" ./target/release/flowerss-bot
# or: ./target/release/flowerss-bot -c config.toml
```

Dry-run (no Telegram sends):

```bash
cargo run -p flowerss-bot -- --dry-run -c config.toml
```

### 5. Migrating from Go flowerss-bot

1. Stop the old bot and back up `data.db`
2. Point `[sqlite].path` at that file
3. `--dry-run`, then start the Rust binary
4. Verify `/list` / `/check` (in polling mode)

Migrations are additive; keep a backup until you trust production.

## Implementation notes

**Included:** command/callback dispatcher (polling), SQLite repo, fetch/dedup pipeline, OPML via settings, HTML send path, 429 `retry_after` handling, graceful shutdown, content retention pruning, optional Telegraph publishing, broadcast mode.

**Limits:** channel `@` preloading and full admin middleware are incomplete — prefer inviting the bot into the target chat/channel and running commands there (or manage under polling, then switch to broadcast). Validate with your real token and DB before cutting over.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT — see [LICENSE](LICENSE).

This work is based on:

- **indes** — original [flowerss-bot](https://github.com/indes/flowerss-bot) (Go)
- **S.Y. Lee** — [tg-kl-vault](https://github.com/siygle/tg-kl-vault) Rust rewrite
- **man smart-home** — this fork (description rendering, localization, Bot API connection modes, channel-oriented defaults)

Third-party crates retain their own licenses.
