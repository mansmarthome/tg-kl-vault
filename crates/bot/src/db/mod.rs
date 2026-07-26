pub mod models;
pub mod repo;

use sqlx::{migrate::Migrator, sqlite::SqlitePoolOptions, Executor, SqlitePool};
use std::{path::Path, time::Duration};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

pub async fn connect(path: &str) -> anyhow::Result<SqlitePool> {
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
