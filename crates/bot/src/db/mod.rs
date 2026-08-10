pub mod bookmarks;
pub mod models;
pub mod repo;

use sqlx::{
    migrate::Migrator,
    sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    },
    SqlitePool,
};
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

    // PRAGMAs live on `SqliteConnectOptions`, not a one-shot `pool.execute`.
    // `synchronous` is per-connection, so the old approach only configured one
    // of the four pooled connections; the tag worker is a second concurrent
    // writer, exactly the condition that turns that into sporadic SQLITE_BUSY.
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?;

    MIGRATOR.run(&pool).await?;
    Ok(pool)
}
