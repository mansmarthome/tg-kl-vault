pub mod models;
pub mod repo;

use sqlx::{migrate::Migrator, sqlite::SqlitePoolOptions, Executor, SqlitePool};
use std::{path::Path, time::Duration};
use tracing::{info, warn};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

pub async fn connect(path: &str) -> anyhow::Result<SqlitePool> {
    // Log the absolute path and whether the file pre-existed so a silently
    // fresh (empty) database — the usual cause of "my subscriptions vanished"
    // after a redeploy onto non-persistent storage — is obvious from the logs.
    let existed = Path::new(path).exists();
    let absolute = std::fs::canonicalize(path)
        .ok()
        .or_else(|| std::env::current_dir().ok().map(|cwd| cwd.join(path)))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| path.to_string());
    if existed {
        info!(db_path = %absolute, "opening existing sqlite database");
    } else {
        warn!(db_path = %absolute, "sqlite database not found; creating a fresh empty one (subscriptions will be empty until re-added)");
    }

    if let Some(parent) = Path::new(path).parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }

    let url = format!("sqlite://{path}?mode=rwc");
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await?;

    pool.execute("PRAGMA journal_mode = WAL").await?;
    pool.execute("PRAGMA busy_timeout = 5000").await?;
    pool.execute("PRAGMA foreign_keys = ON").await?;
    pool.execute("PRAGMA synchronous = NORMAL").await?;

    MIGRATOR.run(&pool).await?;
    Ok(pool)
}
