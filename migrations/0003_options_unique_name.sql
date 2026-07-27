-- Additive index for per-chat settings stored in the legacy options table.
CREATE UNIQUE INDEX IF NOT EXISTS idx_options_name ON options(name);
