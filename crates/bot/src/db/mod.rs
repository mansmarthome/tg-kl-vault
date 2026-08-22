pub mod bookmarks;
pub mod models;
pub mod repo;
pub mod stocks;

use std::{
    collections::HashSet,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use libsql::{Builder, Connection, Database, Row};
use tracing::{info, warn};

/// Result alias for the DB layer. Kept as `libsql::Result` (rather than
/// `anyhow`) so callers can keep `.map_err(to_request_error)` — the libsql
/// error type implements `std::error::Error`, `anyhow::Error` does not.
pub type DbResult<T> = libsql::Result<T>;

/// Replacement for `sqlx::FromRow`: build a model from a positional libsql row.
/// Column order must match the `SELECT` list in the query.
pub trait FromRow: Sized {
    fn from_row(row: &Row) -> libsql::Result<Self>;
}

impl FromRow for (i64, i64) {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok((row.get::<i64>(0)?, row.get::<i64>(1)?))
    }
}

impl FromRow for (String, i64) {
    fn from_row(row: &Row) -> libsql::Result<Self> {
        Ok((row.get::<String>(0)?, row.get::<i64>(1)?))
    }
}

/// Owns the libsql `Database` (kept alive so the embedded-replica background
/// sync task keeps running) plus one shared `Connection`. `Connection` is
/// cheap to clone and serializes access internally, so a single one is enough
/// for this bot's write-mostly, low-concurrency workload.
pub struct Db {
    _database: Database,
    conn: Connection,
    remote: bool,
}

impl Db {
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// True when running as a Turso embedded replica (remote primary + local
    /// file). Used by the caller to decide whether periodic `sync()` applies.
    pub fn is_remote(&self) -> bool {
        self.remote
    }

    pub async fn sync(&self) -> anyhow::Result<()> {
        if self.remote {
            self._database.sync().await.context("libsql sync")?;
        }
        Ok(())
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// libsql embedded replicas pair the db file with a `{path}-info` metadata
/// sidecar. Opening a pre-existing plain sqlite file (no sidecar) fails with
/// "db file exists but metadata file does not". When we hit that orphan state,
/// rename the local file(s) aside so the replica can bootstrap from remote.
///
/// The backup is kept so operators can import it into Turso if the remote was
/// empty and the local file still held production data.
fn prepare_local_path_for_replica(path: &str) -> anyhow::Result<()> {
    let db = Path::new(path);
    let info_path = format!("{path}-info");
    let info = Path::new(&info_path);
    let db_exists = db.exists();
    let info_exists = info.exists();

    match (db_exists, info_exists) {
        // Fresh start or already a valid replica — nothing to do.
        (false, false) | (true, true) => Ok(()),
        // Orphan metadata without db: drop the sidecar so bootstrap can run.
        (false, true) => {
            warn!(
                metadata = %info.display(),
                "found orphan replica metadata without db file; removing so bootstrap can proceed"
            );
            std::fs::remove_file(info)
                .with_context(|| format!("remove orphan metadata {}", info.display()))?;
            Ok(())
        }
        // Plain local sqlite (or half-broken replica): move aside.
        (true, false) => {
            let stamp = now_unix();
            let backup_path = format!("{path}.pre-turso.{stamp}.bak");
            let backup = Path::new(&backup_path);
            warn!(
                db_path = %db.display(),
                backup = %backup.display(),
                "local sqlite has no replica metadata (-info); moving it aside so Turso can bootstrap a fresh replica. If the remote primary is empty, import this backup into Turso before relying on cloud data."
            );
            std::fs::rename(db, backup)
                .with_context(|| format!("rename {} -> {}", db.display(), backup.display()))?;
            // Best-effort: also park WAL/SHM companions if present.
            for suffix in ["-wal", "-shm"] {
                let side_path = format!("{path}{suffix}");
                let side = Path::new(&side_path);
                if side.exists() {
                    let side_bak_path = format!("{path}{suffix}.pre-turso.{stamp}.bak");
                    let side_bak = Path::new(&side_bak_path);
                    let _ = std::fs::rename(side, side_bak);
                }
            }
            Ok(())
        }
    }
}

/// Opens the database. Two modes, chosen by env so existing local-file
/// deployments keep working with zero configuration:
///
/// * `TURSO_DATABASE_URL` set  -> embedded replica (write-through to the Turso
///   primary, reads served locally, background sync every 60s).
/// * otherwise                 -> a plain local libsql file at `path`.
pub async fn connect(path: &str) -> anyhow::Result<Arc<Db>> {
    // Log the absolute path and whether the file pre-existed so a silently
    // fresh (empty) database — the usual cause of "my subscriptions vanished"
    // after a redeploy onto non-persistent storage — is obvious from the logs.
    let existed = Path::new(path).exists();
    let absolute = std::fs::canonicalize(path)
        .ok()
        .or_else(|| std::env::current_dir().ok().map(|cwd| cwd.join(path)))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| path.to_string());

    if let Some(parent) = Path::new(path).parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }

    let turso_url = std::env::var("TURSO_DATABASE_URL").ok().filter(|u| !u.is_empty());
    let remote = turso_url.is_some();

    let database = match turso_url {
        Some(url) => {
            // Embedded replicas keep a sidecar `{path}-info` metadata file. A
            // plain local sqlite left over from pre-Turso deploys has the db
            // file but no metadata, and libsql refuses to open that state with
            // "db file exists but metadata file does not". Move the orphan
            // aside so we can bootstrap a fresh replica from the remote primary.
            prepare_local_path_for_replica(path)?;
            let token = std::env::var("TURSO_AUTH_TOKEN").unwrap_or_default();
            info!(db_path = %absolute, "opening Turso embedded replica (write-through to remote primary)");
            Builder::new_remote_replica(path, url, token)
                .sync_interval(Duration::from_secs(60))
                .build()
                .await
                .context("open Turso embedded replica")?
        }
        None => {
            if existed {
                info!(db_path = %absolute, "opening existing local sqlite database");
            } else {
                warn!(db_path = %absolute, "sqlite database not found; creating a fresh empty one (subscriptions will be empty until re-added)");
            }
            Builder::new_local(path).build().await.context("open local sqlite database")?
        }
    };

    let conn = database.connect().context("open db connection")?;
    conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;")
        .await
        .context("apply pragmas")?;

    let db = Db { _database: database, conn, remote };
    if remote {
        db.sync().await?;
    }
    run_migrations(&db.conn).await.context("run migrations")?;
    Ok(Arc::new(db))
}

/// Embedded SQL migrations, keyed by the numeric version sqlx assigned them
/// (the `NNNN_` filename prefix parsed as an integer). Embedding via
/// `include_str!` means the running binary never depends on the on-disk
/// `migrations/` directory.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../../../../migrations/0001_go_schema.sql")),
    (2, include_str!("../../../../migrations/0002_rust_additive.sql")),
    (3, include_str!("../../../../migrations/0003_options_unique_name.sql")),
    (4, include_str!("../../../../migrations/0004_bookmarks.sql")),
    (5, include_str!("../../../../migrations/0005_source_health.sql")),
    (6, include_str!("../../../../migrations/0006_stocks.sql")),
];

async fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _kl_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);",
    )
    .await?;

    let mut applied = read_versions(conn, "SELECT version FROM _kl_migrations").await;
    // Databases created by the previous sqlx-based code already ran these
    // migrations and recorded them in `_sqlx_migrations`. Honour that so we
    // don't re-run non-idempotent steps (e.g. 0002's `ADD COLUMN`).
    applied.extend(read_versions(conn, "SELECT version FROM _sqlx_migrations").await);

    for (version, sql) in MIGRATIONS {
        if applied.contains(version) {
            continue;
        }
        conn.execute_batch(sql)
            .await
            .with_context(|| format!("apply migration {version}"))?;
        conn.execute(
            "INSERT OR IGNORE INTO _kl_migrations (version, applied_at) VALUES (?, ?)",
            libsql::params![*version, now_unix()],
        )
        .await?;
        info!(version, "applied db migration");
    }
    Ok(())
}

/// Reads a set of migration versions; a missing table (the query errors) is
/// treated as "none applied".
async fn read_versions(conn: &Connection, sql: &str) -> HashSet<i64> {
    let mut out = HashSet::new();
    let Ok(mut rows) = conn.query(sql, ()).await else {
        return out;
    };
    while let Ok(Some(row)) = rows.next().await {
        if let Ok(v) = row.get::<i64>(0) {
            out.insert(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn prepare_replica_moves_orphan_plain_sqlite_aside() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        fs::write(&db_path, b"sqlite-bytes").unwrap();
        fs::write(dir.path().join("data.db-wal"), b"wal").unwrap();

        prepare_local_path_for_replica(db_path.to_str().unwrap()).unwrap();

        assert!(!db_path.exists(), "orphan db must be moved aside");
        assert!(!dir.path().join("data.db-wal").exists());
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".pre-turso."))
            .collect();
        assert!(backups.iter().any(|n| n.starts_with("data.db.pre-turso.")));
        assert!(backups.iter().any(|n| n.starts_with("data.db-wal.pre-turso.")));
    }

    #[test]
    fn prepare_replica_noop_when_pair_is_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        fs::write(&db_path, b"sqlite-bytes").unwrap();
        let info_path = format!("{}-info", db_path.display());
        fs::write(&info_path, b"{}").unwrap();

        prepare_local_path_for_replica(db_path.to_str().unwrap()).unwrap();

        assert!(db_path.exists());
        assert!(Path::new(&info_path).exists());
    }

    #[test]
    fn prepare_replica_drops_orphan_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let info = dir.path().join("data.db-info");
        fs::write(&info, b"{}").unwrap();

        prepare_local_path_for_replica(db_path.to_str().unwrap()).unwrap();

        assert!(!info.exists());
        assert!(!db_path.exists());
    }

    /// A production `data.db` predates migration 0005. Opening it must add the
    /// health columns in place, keep every existing row, and be a no-op the
    /// second time — `ALTER TABLE ADD COLUMN` would error if it ever re-ran.
    #[tokio::test]
    async fn legacy_database_gains_the_health_columns_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let path = db_path.to_str().unwrap();

        // Stand up a pre-0005 database the way an existing deployment has it:
        // migrations 1-4 applied and recorded.
        {
            let db = connect(path).await.unwrap();
            db.conn
                .execute_batch(
                    "ALTER TABLE sources DROP COLUMN last_error; \
                     ALTER TABLE sources DROP COLUMN last_error_at; \
                     ALTER TABLE sources DROP COLUMN last_success_at; \
                     DELETE FROM _kl_migrations WHERE version = 5; \
                     INSERT INTO sources (link, title, error_count, next_fetch_at) \
                       VALUES ('https://old.test/feed', 'Legacy Feed', 3, 0);",
                )
                .await
                .unwrap();
        }

        for pass in 1..=2 {
            let db = connect(path).await.unwrap();
            let mut rows = db
                .conn
                .query(
                    "SELECT title, error_count, last_error, last_success_at FROM sources",
                    (),
                )
                .await
                .unwrap_or_else(|e| panic!("pass {pass} must open the upgraded db: {e}"));
            let row = rows.next().await.unwrap().expect("legacy row must survive");
            assert_eq!(row.get::<String>(0).unwrap(), "Legacy Feed");
            assert_eq!(row.get::<i64>(1).unwrap(), 3);
            assert_eq!(row.get::<Option<String>>(2).unwrap(), None);
            assert_eq!(row.get::<Option<i64>>(3).unwrap(), None);
            assert!(rows.next().await.unwrap().is_none(), "no rows duplicated");
        }
    }

    /// A database predating migration 0006 must gain the stock tables on open,
    /// exactly once — opening twice must not error (a second `CREATE TABLE`
    /// without `IF NOT EXISTS` would, so this also guards that the SQL is
    /// re-runnable) and must preserve any rows written between opens.
    #[tokio::test]
    async fn legacy_database_gains_the_stock_tables_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let path = db_path.to_str().unwrap();

        // Stand up a pre-0006 database: drop the stock tables and un-record 6.
        {
            let db = connect(path).await.unwrap();
            db.conn
                .execute_batch(
                    "DROP TABLE stock_watchlist; DROP TABLE stock_meta; \
                     DROP TABLE stock_bars; DROP TABLE stock_push_settings; \
                     DROP TABLE stock_report_log; DROP TABLE stock_commentary; \
                     DELETE FROM _kl_migrations WHERE version = 6;",
                )
                .await
                .unwrap();
        }

        // First reopen applies 0006 and lets us insert a row.
        {
            let db = connect(path).await.unwrap();
            db.conn
                .execute(
                    "INSERT INTO stock_watchlist \
                     (chat_id, created_by, symbol, market, created_at, updated_at) \
                     VALUES (1, 1, '2330.TW', 'tw', 0, 0)",
                    (),
                )
                .await
                .unwrap();
        }

        // Second reopen must be a no-op that keeps the row.
        {
            let db = connect(path).await.unwrap();
            let n: i64 = db
                .conn
                .query("SELECT COUNT(*) FROM stock_watchlist", ())
                .await
                .unwrap()
                .next()
                .await
                .unwrap()
                .unwrap()
                .get(0)
                .unwrap();
            assert_eq!(n, 1, "the row must survive the second open");
        }
    }
}
