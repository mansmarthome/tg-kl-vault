-- Byte-compatible baseline for the Go/GORM SQLite schema.
-- Do not rename/drop columns; existing production data.db files must open as-is.
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    link TEXT,
    title TEXT,
    error_count INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS subscribes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER,
    source_id INTEGER,
    enable_notification INTEGER,
    enable_telegraph INTEGER,
    tag TEXT,
    interval INTEGER,
    wait_time INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS contents (
    source_id INTEGER,
    hash_id TEXT PRIMARY KEY,
    raw_id TEXT,
    raw_link TEXT,
    title TEXT,
    telegraph_url TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS options (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    value TEXT,
    created_at TEXT,
    updated_at TEXT
);
