-- Sanctioned deviations D2/D3/D4/D5: additive-only metadata and indexes.
-- SQLite in the resolved sqlx/libsqlite stack rejects `ADD COLUMN IF NOT EXISTS`.
-- SQLx records applied migrations, so these additive statements run once.
ALTER TABLE sources ADD COLUMN etag TEXT;
ALTER TABLE sources ADD COLUMN last_modified TEXT;
ALTER TABLE sources ADD COLUMN next_fetch_at INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_sources_next_fetch ON sources(next_fetch_at);
CREATE INDEX IF NOT EXISTS idx_contents_source     ON contents(source_id, created_at);
CREATE INDEX IF NOT EXISTS idx_subscribes_source   ON subscribes(source_id);
CREATE INDEX IF NOT EXISTS idx_subscribes_user     ON subscribes(user_id);
