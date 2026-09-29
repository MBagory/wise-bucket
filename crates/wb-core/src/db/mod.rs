//! Database access: connection, requirements, migrations and lifecycle.

pub mod backup;
pub mod managed;

use std::time::Duration;

use serde::Serialize;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgPool};

use crate::config::{Database, EffectiveConfig};
use crate::error::{ErrorKind, Result, err};
use crate::fsutil::FileLock;
use managed::{Managed, connect_err, db_err};

/// `application_name` of MCP server sessions (used to count live sessions).
pub const SESSION_APP_NAME: &str = "wisebucket";
/// `application_name` of short-lived CLI connections (never counted as sessions).
pub const CLI_APP_NAME: &str = "wisebucket-cli";

/// Minimum supported PostgreSQL version (`server_version_num`).
pub const MIN_SERVER_VERSION_NUM: i32 = 150000;
/// Minimum supported pgvector version.
pub const MIN_VECTOR_VERSION: (u64, u64) = (0, 8);

/// Why a connection is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// A long-lived MCP server session.
    Session,
    /// A short CLI command.
    Cli,
}

impl Purpose {
    fn app_name(self) -> &'static str {
        match self {
            Purpose::Session => SESSION_APP_NAME,
            Purpose::Cli => CLI_APP_NAME,
        }
    }
}

/// Facts about the connected database.
#[derive(Debug, Clone, Serialize)]
pub struct DbInfo {
    pub mode: &'static str,
    pub server_version: String,
    pub server_version_num: i32,
    pub vector_version: Option<String>,
    pub pg_trgm_version: Option<String>,
    pub database: String,
    pub user: String,
}

/// An open database handle.
pub struct Db {
    pub pool: PgPool,
    pub info: DbInfo,
    managed: Option<Managed>,
    purpose: Purpose,
    /// `true` if this process started the managed server.
    pub started_here: bool,
}

/// Returns the managed-mode helper for a configuration, if applicable.
pub fn managed_for(cfg: &EffectiveConfig) -> Option<Managed> {
    matches!(cfg.database.value, Database::Managed)
        .then(|| Managed::new(&cfg.state_dir.value, &cfg.runtime_dir.value))
}

fn connect_options(cfg: &EffectiveConfig, purpose: Purpose) -> Result<PgConnectOptions> {
    let opts = match &cfg.database.value {
        Database::Managed => managed_for(cfg)
            .ok_or_else(|| err(ErrorKind::Internal, "managed mode expected"))?
            .app_options()?,
        Database::External { url } => url.parse::<PgConnectOptions>().map_err(|e| {
            err(
                ErrorKind::ConfigInvalid,
                format!("invalid database URL: {e}"),
            )
        })?,
    };
    Ok(opts
        .application_name(purpose.app_name())
        .log_statements(tracing::log::LevelFilter::Debug)
        .log_slow_statements(tracing::log::LevelFilter::Info, Duration::from_secs(5)))
}

/// Opens the database: starts the managed server if needed, checks requirements and migrates.
pub async fn open(cfg: &EffectiveConfig, purpose: Purpose) -> Result<Db> {
    let managed = managed_for(cfg);
    let mut started_here = false;
    let pool;
    if let Some(m) = &managed {
        if !m.is_initialized() {
            return Err(err(
                ErrorKind::DbNotInitialized,
                format!("no managed database in {}", cfg.state_dir.value.display()),
            ));
        }
        // Start and connect under the lifecycle lock so a concurrent "stop if last" cannot interleave.
        let _lock = FileLock::acquire(&m.lifecycle_lock_path())?;
        if !m.is_running()? {
            m.start()?;
            started_here = true;
        }
        pool = pool_for(cfg, purpose).await?;
    } else {
        pool = pool_for(cfg, purpose).await?;
    }
    let info = inspect(&pool, cfg).await?;
    check_requirements(&info)?;
    migrate(&pool).await?;
    Ok(Db {
        pool,
        info,
        managed,
        purpose,
        started_here,
    })
}

async fn pool_for(cfg: &EffectiveConfig, purpose: Purpose) -> Result<PgPool> {
    let opts = connect_options(cfg, purpose)?;
    let (min, max) = match purpose {
        // Keep one connection open for the whole session: it is how other
        // processes know this session is alive (see `release`).
        Purpose::Session => (1, 5),
        Purpose::Cli => (1, 2),
    };
    PgPoolOptions::new()
        .min_connections(min)
        .max_connections(max)
        .idle_timeout(None)
        .max_lifetime(None)
        .acquire_timeout(Duration::from_secs(20))
        .connect_with(opts)
        .await
        .map_err(connect_err)
}

/// Reads versions and extensions of the connected database.
pub async fn inspect(pool: &PgPool, cfg: &EffectiveConfig) -> Result<DbInfo> {
    let (server_version, num, database, user): (String, String, String, String) = sqlx::query_as(
        "SELECT current_setting('server_version'), current_setting('server_version_num'), current_database(), current_user",
    )
    .fetch_one(pool)
    .await
    .map_err(db_err)?;
    let ext = |name: &'static str| async move {
        // Installed version if the extension exists, otherwise the version available to install.
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT coalesce(
                (SELECT extversion FROM pg_extension WHERE extname = $1),
                (SELECT default_version FROM pg_available_extensions WHERE name = $1))",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .map_err(db_err)
    };
    Ok(DbInfo {
        mode: cfg.database.value.mode_name(),
        server_version,
        server_version_num: num.parse().unwrap_or(0),
        vector_version: ext("vector").await?,
        pg_trgm_version: ext("pg_trgm").await?,
        database,
        user,
    })
}

fn version_at_least(v: &str, min: (u64, u64)) -> bool {
    let mut it = v.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    let (major, minor) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
    (major, minor) >= min
}

/// Verifies server version and extension availability.
pub fn check_requirements(info: &DbInfo) -> Result<()> {
    if info.server_version_num < MIN_SERVER_VERSION_NUM {
        return Err(err(
            ErrorKind::DbVersionUnsupported,
            format!(
                "PostgreSQL {} found; 15 or newer is required",
                info.server_version
            ),
        ));
    }
    match &info.vector_version {
        Some(v) if version_at_least(v, MIN_VECTOR_VERSION) => {}
        Some(v) => {
            return Err(err(
                ErrorKind::VectorExtensionMissing,
                format!("pgvector {v} found; 0.8 or newer is required"),
            ));
        }
        None => {
            return Err(err(
                ErrorKind::VectorExtensionMissing,
                "the `vector` extension is not available on this server",
            ));
        }
    }
    if info.pg_trgm_version.is_none() {
        return Err(err(
            ErrorKind::VectorExtensionMissing,
            "the `pg_trgm` extension (PostgreSQL contrib) is not available on this server",
        ));
    }
    Ok(())
}

/// Embedded schema migrations (`crates/wb-core/migrations`).
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Applies pending migrations (concurrent callers are serialized by sqlx's advisory lock).
pub async fn migrate(pool: &PgPool) -> Result<()> {
    MIGRATOR.run(pool).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("permission denied") {
            err(
                ErrorKind::DbPrivilegesInsufficient,
                format!("{msg}. A superuser may need to run `CREATE EXTENSION vector; CREATE EXTENSION pg_trgm;` in this database"),
            )
        } else {
            err(ErrorKind::MigrationFailed, msg)
        }
    })
}

/// What happened to the managed server when a handle was released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseOutcome {
    /// External database: nothing to do.
    External,
    /// `keep_running` is set.
    KeptRunning,
    /// Other sessions are still connected.
    OtherSessionsActive,
    /// This was the last session: the server was stopped.
    Stopped,
}

impl Db {
    /// Closes the pool and, in managed mode, stops PostgreSQL if no session remains.
    ///
    /// A CLI command only stops the server if it started it itself, so
    /// `db start` followed by `backup` leaves the server running.
    pub async fn release(&self, cfg: &EffectiveConfig) -> Result<ReleaseOutcome> {
        self.pool.close().await;
        let Some(m) = &self.managed else {
            return Ok(ReleaseOutcome::External);
        };
        if cfg.keep_running.value || (self.purpose == Purpose::Cli && !self.started_here) {
            return Ok(ReleaseOutcome::KeptRunning);
        }
        stop_if_idle(m).await
    }

    pub fn purpose(&self) -> Purpose {
        self.purpose
    }
}

/// Stops the managed server if no MCP session is connected (under the lifecycle lock).
pub async fn stop_if_idle(m: &Managed) -> Result<ReleaseOutcome> {
    use sqlx::Connection;
    let _lock = FileLock::acquire(&m.lifecycle_lock_path())?;
    if !m.is_running()? {
        return Ok(ReleaseOutcome::Stopped);
    }
    let mut conn = sqlx::PgConnection::connect_with(
        &m.app_options()?.application_name("wisebucket-lifecycle"),
    )
    .await
    .map_err(connect_err)?;
    let others = count_session_backends(&mut conn).await?;
    conn.close().await.ok();
    if others > 0 {
        return Ok(ReleaseOutcome::OtherSessionsActive);
    }
    m.stop()?;
    Ok(ReleaseOutcome::Stopped)
}

/// Number of backends belonging to live MCP sessions.
pub async fn count_session_backends(conn: &mut sqlx::PgConnection) -> Result<i64> {
    sqlx::query_scalar(
        "SELECT count(DISTINCT pid)::bigint FROM pg_stat_activity WHERE application_name = $1",
    )
    .bind(SESSION_APP_NAME)
    .fetch_one(conn)
    .await
    .map_err(db_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(version_at_least("0.8.6", (0, 8)));
        assert!(version_at_least("1.0", (0, 8)));
        assert!(!version_at_least("0.7.4", (0, 8)));
    }
}
