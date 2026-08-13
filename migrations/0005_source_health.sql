-- Source health history, so a feed that quietly stopped working is visible in
-- the bot instead of only in the server logs. `mark_source_error` previously
-- bumped `error_count` and threw the error text away.
--
-- Additive only, same constraints as 0002: no renames/drops, and SQLite in this
-- stack rejects `ADD COLUMN IF NOT EXISTS`. Applied migrations are recorded in
-- `_kl_migrations`, so these run exactly once.
ALTER TABLE sources ADD COLUMN last_error TEXT;
ALTER TABLE sources ADD COLUMN last_error_at INTEGER;
ALTER TABLE sources ADD COLUMN last_success_at INTEGER;
