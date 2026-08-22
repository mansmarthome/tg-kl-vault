-- Stock tracking. Purely additive, same rules as 0004: no renames, no drops,
-- no `ADD COLUMN IF NOT EXISTS`. Timestamps are INTEGER unix seconds (0002/0004
-- convention); the one exception is a trading day, which is a foreign-timezone
-- *calendar date* rather than an instant, stored as TEXT 'YYYY-MM-DD'.

-- Watchlist (自選股清單).
CREATE TABLE IF NOT EXISTS stock_watchlist (
  -- AUTOINCREMENT for the same reason as 0004: the id goes into callback_data,
  -- and those buttons outlive the process. rowid reuse would let a stale 🗑
  -- button delete a *different* stock.
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_id      INTEGER NOT NULL,
  created_by   INTEGER NOT NULL,
  symbol       TEXT    NOT NULL,            -- canonical: 2330.TW / 6488.TWO / AAPL
  market       TEXT    NOT NULL,            -- 'tw' | 'us'   scheduling bucket
  exchange     TEXT    NOT NULL DEFAULT '', -- meta.exchangeName: TAI/TWO/NMS/NYQ
  display_name TEXT    NOT NULL DEFAULT '', -- snapshot at add time; list renders when Yahoo is down
  currency     TEXT    NOT NULL DEFAULT '',
  note         TEXT    NOT NULL DEFAULT '',
  sort_order   INTEGER NOT NULL DEFAULT 0,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_stock_watchlist_chat_symbol
  ON stock_watchlist(chat_id, symbol);           -- dedup key + upsert target
CREATE INDEX IF NOT EXISTS idx_stock_watchlist_chat_market
  ON stock_watchlist(chat_id, market, sort_order, id);  -- covers the paged list
CREATE INDEX IF NOT EXISTS idx_stock_watchlist_market_symbol
  ON stock_watchlist(market, symbol);            -- worker's SELECT DISTINCT symbol

-- Per-symbol meta / session-clock cache (one row per symbol).
CREATE TABLE IF NOT EXISTS stock_meta (
  symbol        TEXT PRIMARY KEY,
  market        TEXT NOT NULL,
  exchange      TEXT NOT NULL DEFAULT '',
  display_name  TEXT NOT NULL DEFAULT '',
  currency      TEXT NOT NULL DEFAULT '',
  gmtoffset     INTEGER NOT NULL DEFAULT 0,      -- observed offset on each response
  session_start INTEGER,                         -- currentTradingPeriod.regular.start
  session_end   INTEGER,                         -- .end — the close gate
  market_time   INTEGER,                         -- meta.regularMarketTime
  last_price    REAL,
  prev_close    REAL,
  week52_high   REAL,
  week52_low    REAL,
  trade_date    TEXT NOT NULL DEFAULT '',        -- latest bar's market-local date
  source        TEXT NOT NULL DEFAULT '',
  fetched_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

-- OHLCV, normalized one row per day. Beats a JSON blob: per-row source tagging,
-- prune_stock_bars (cf. prune_contents), and range queries pushed down to SQL.
CREATE TABLE IF NOT EXISTS stock_bars (
  symbol     TEXT    NOT NULL,
  trade_date TEXT    NOT NULL,                   -- 'YYYY-MM-DD' market-local
  ts         INTEGER NOT NULL,                   -- bar start epoch
  open       REAL, high REAL, low REAL, close REAL,
  volume     INTEGER,
  source     TEXT    NOT NULL DEFAULT '',        -- 'yahoo'|'twse'|'tpex' per-row provenance
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (symbol, trade_date)               -- natural upsert key
);
CREATE INDEX IF NOT EXISTS idx_stock_bars_symbol_ts ON stock_bars(symbol, ts DESC);

-- Per-chat, per-market close-push settings.
CREATE TABLE IF NOT EXISTS stock_push_settings (
  chat_id     INTEGER NOT NULL,
  market      TEXT    NOT NULL,                  -- 'tw' | 'us'
  enabled     INTEGER NOT NULL DEFAULT 1,
  -- Minutes from market-local midnight, 0..1439 — not "14:30". The scheduler
  -- compares it against a computed local-minute every pass: no string parsing
  -- on the hot path, and the column can't hold "25:99". NULL = "default delay
  -- after the close".
  push_minute INTEGER,
  updated_at  INTEGER NOT NULL,
  PRIMARY KEY (chat_id, market)
);

-- Send-dedup ledger.
CREATE TABLE IF NOT EXISTS stock_report_log (
  chat_id    INTEGER NOT NULL,
  market     TEXT    NOT NULL,
  -- Market-local trading day from *data*, never the local clock. This is why
  -- weekends and holidays are free: on a non-trading day the latest bar is
  -- still the previous session, whose row already exists, so the whole pass is
  -- a no-op. No calendar to maintain means no calendar to rot.
  trade_date TEXT    NOT NULL,
  status     INTEGER NOT NULL DEFAULT 0,         -- 0 claimed, 1 sent, 2 forbidden
  attempts   INTEGER NOT NULL DEFAULT 0,
  claimed_at INTEGER NOT NULL,
  sent_at    INTEGER,
  PRIMARY KEY (chat_id, market, trade_date)
);
-- Partial index covering only the "claimed but not sent" recovery query.
-- status=1 is 99.9% of rows and must not be indexed — same shape as 0004's
-- idx_bookmarks_pending.
CREATE INDEX IF NOT EXISTS idx_stock_report_log_stuck
  ON stock_report_log(claimed_at) WHERE status = 0;

-- AI commentary cache. Keyed with lang: same numbers, two narrative languages.
-- 50 chats tracking the same 2330 pay the AI cost once.
CREATE TABLE IF NOT EXISTS stock_commentary (
  symbol     TEXT NOT NULL,
  trade_date TEXT NOT NULL,
  lang       TEXT NOT NULL,                      -- 'zh-tw' | 'en'
  body       TEXT NOT NULL,
  provider   TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  PRIMARY KEY (symbol, trade_date, lang)
);
