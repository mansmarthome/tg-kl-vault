-- Per-chat bookmark library + AI auto-tagging (design 2026-08-06).
-- Purely additive. Timestamps are INTEGER unix seconds, following 0002's
-- precedent (not 0001's TEXT).

CREATE TABLE IF NOT EXISTS bookmarks (
  -- AUTOINCREMENT is required: `id` goes into inline-keyboard callback_data,
  -- and those buttons outlive the process in users' chat history. Without it
  -- SQLite reuses rowids, so a stale button would edit a *different* bookmark.
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_id             INTEGER NOT NULL,           -- ownership
  created_by          INTEGER NOT NULL,           -- creator (group delete rule)
  url                 TEXT    NOT NULL,           -- normalized; also the dedupe key
  title               TEXT    NOT NULL DEFAULT '',
  note                TEXT    NOT NULL DEFAULT '',
  source_title        TEXT    NOT NULL DEFAULT '', -- snapshot; `sources` may be deleted
  content_hash_id     TEXT,                       -- breadcrumb only; may dangle, never JOINed
  telegraph_url       TEXT,
  tag_state           INTEGER NOT NULL DEFAULT 0, -- 0 pending, 1 done
  tag_attempts        INTEGER NOT NULL DEFAULT 0,
  tag_next_attempt_at INTEGER NOT NULL DEFAULT 0,
  notify_message_id   INTEGER,                    -- message the worker edits; NULL = nothing to edit
  notify_kind         INTEGER NOT NULL DEFAULT 0, -- 0 = edit text+keyboard, 1 = keyboard label only
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_bookmarks_chat_url ON bookmarks(chat_id, url);
CREATE INDEX IF NOT EXISTS idx_bookmarks_chat_id_desc   ON bookmarks(chat_id, id);
CREATE INDEX IF NOT EXISTS idx_bookmarks_pending
  ON bookmarks(tag_next_attempt_at, id) WHERE tag_state = 0;

CREATE TABLE IF NOT EXISTS bookmark_tags (
  -- Deliberately no FK: once `foreign_keys` is genuinely on per connection
  -- (0-step / db/mod.rs), a missing ON DELETE CASCADE would make bookmark
  -- deletes start failing. Tag rows are deleted explicitly in the same
  -- transaction instead.
  bookmark_id INTEGER NOT NULL,
  tag         TEXT    NOT NULL,
  origin      INTEGER NOT NULL DEFAULT 0,  -- 0 = ai, 1 = manual
  PRIMARY KEY (bookmark_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_bookmark_tags_tag ON bookmark_tags(tag);
