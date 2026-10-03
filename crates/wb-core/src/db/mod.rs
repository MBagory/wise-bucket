//! Database access: connection, requirements, migrations and lifecycle.

pub mod backup;
pub mod managed;

use std::time::Duration;

use serde::Serialize;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgPool};

use crate::config::EffectiveConfig;
use crate::error::{ErrorKind, Result, err};
use crate::fsutil::FileLock;
use managed::{Managed, connect_err, db_err};

/// `application_name` of MCP server sessions (used to count live sessions).
pub const SESSION_APP_NAME: &str = "wisebucket";
/// `application_name` of short-lived CLI connections (never counted as sessions).
pub const CLI_APP_NAME: &str = "wisebucket-cli";

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
    pub server_version: String,
    pub server_version_num: i32,
    pub pg_trgm_version: Option<String>,
    pub database: String,
    pub user: String,
}

/// An open database handle.
pub struct Db {
    pub pool: PgPool,
    pub info: DbInfo,
    managed: Managed,
    purpose: Purpose,
    /// `true` if this process started the managed server.
    pub started_here: bool,
}

/// Returns the managed PostgreSQL helper for a configuration.
pub fn managed_for(cfg: &EffectiveConfig) -> Managed {
    Managed::new(&cfg.state_dir.value, &cfg.runtime_dir.value)
}

fn connect_options(cfg: &EffectiveConfig, purpose: Purpose) -> Result<PgConnectOptions> {
    Ok(managed_for(cfg)
        .app_options()?
        .application_name(purpose.app_name())
        .log_statements(tracing::log::LevelFilter::Debug)
        .log_slow_statements(tracing::log::LevelFilter::Info, Duration::from_secs(5)))
}

/// Opens the database: starts the managed server if needed and migrates.
pub async fn open(cfg: &EffectiveConfig, purpose: Purpose) -> Result<Db> {
    let managed = managed_for(cfg);
    if !managed.is_initialized() {
        return Err(err(
            ErrorKind::DbNotInitialized,
            format!("no managed database in {}", cfg.state_dir.value.display()),
        ));
    }
    let mut started_here = false;
    let pool = {
        // Start and connect under the lifecycle lock so a concurrent "stop if last" cannot interleave.
        let _lock = FileLock::acquire(&managed.lifecycle_lock_path())?;
        if !managed.is_running()? {
            managed.start()?;
            started_here = true;
        }
        pool_for(cfg, purpose).await?
    };
    let info = inspect(&pool).await?;
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
pub async fn inspect(pool: &PgPool) -> Result<DbInfo> {
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
        server_version,
        server_version_num: num.parse().unwrap_or(0),
        pg_trgm_version: ext("pg_trgm").await?,
        database,
        user,
    })
}

/// Embedded schema migrations (`crates/wb-core/migrations`).
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Applies pending migrations (concurrent callers are serialized by sqlx's advisory lock).
pub async fn migrate(pool: &PgPool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|e| err(ErrorKind::MigrationFailed, e.to_string()))
}

/// What happened to the managed server when a handle was released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseOutcome {
    /// `keep_running` is set.
    KeptRunning,
    /// Other sessions are still connected.
    OtherSessionsActive,
    /// This was the last session: the server was stopped.
    Stopped,
}

impl Db {
    /// Closes the pool and stops PostgreSQL if no session remains.
    ///
    /// A CLI command only stops the server if it started it itself, so
    /// `db start` followed by `backup` leaves the server running.
    pub async fn release(&self, cfg: &EffectiveConfig) -> Result<ReleaseOutcome> {
        self.pool.close().await;
        if cfg.keep_running.value || (self.purpose == Purpose::Cli && !self.started_here) {
            return Ok(ReleaseOutcome::KeptRunning);
        }
        stop_if_idle(&self.managed).await
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
