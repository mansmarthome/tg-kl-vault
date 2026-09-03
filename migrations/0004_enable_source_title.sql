-- Per-subscription opt-out for the bold source-title header in feed posts.
-- Defaults to 1 so existing rows match today's "always show" behaviour.
ALTER TABLE subscribes ADD COLUMN enable_source_title INTEGER NOT NULL DEFAULT 1;