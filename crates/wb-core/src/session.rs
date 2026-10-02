//! MCP sessions: one row per server process, used for provenance and liveness.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::managed::db_err;
use crate::error::Result;

/// A session is considered active if it was seen within this many seconds.
pub const ACTIVE_WINDOW_SECS: i64 = 90;
/// How often a running server refreshes `last_seen_at`.
pub const HEARTBEAT_SECS: u64 = 30;

/// Identifier of a session (UUID v7, time-ordered).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Facts recorded when a session starts.
#[derive(Debug, Clone, Default)]
pub struct SessionStart {
    pub pid: u32,
    pub hostname: Option<String>,
    pub cwd: Option<String>,
}

pub async fn start(pool: &PgPool, s: &SessionStart) -> Result<SessionId> {
    let id = SessionId::new();
    sqlx::query(
        "INSERT INTO wb.session (id, pid, hostname, cwd, wb_version) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id.0)
    .bind(s.pid as i32)
    .bind(&s.hostname)
    .bind(&s.cwd)
    .bind(crate::VERSION)
    .execute(pool)
    .await
    .map_err(db_err)?;
    Ok(id)
}

/// Records the MCP client (from `initialize`).
pub async fn set_client(pool: &PgPool, id: SessionId, name: &str, version: &str) -> Result<()> {
    sqlx::query(
        "UPDATE wb.session SET client_name = $2, client_version = $3, last_seen_at = now(), rev = nextval('wb.rev_seq') WHERE id = $1",
    )
    .bind(id.0)
    .bind(name)
    .bind(version)
    .execute(pool)
    .await
    .map_err(db_err)?;
    Ok(())
}

pub async fn heartbeat(pool: &PgPool, id: SessionId) -> Result<()> {
    sqlx::query("UPDATE wb.session SET last_seen_at = now() WHERE id = $1")
        .bind(id.0)
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}

pub async fn end(pool: &PgPool, id: SessionId) -> Result<()> {
    sqlx::query("UPDATE wb.session SET ended_at = now(), last_seen_at = now(), rev = nextval('wb.rev_seq') WHERE id = $1")
        .bind(id.0)
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// A session as shown to users.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SessionRow {
    pub id: Uuid,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
    pub pid: i32,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
}

/// Sessions that have not ended and were seen recently.
pub async fn active(pool: &PgPool) -> Result<Vec<SessionRow>> {
    sqlx::query_as(
        "SELECT id, client_name, client_version, pid, started_at, last_seen_at
         FROM wb.session
         WHERE ended_at IS NULL AND last_seen_at > now() - make_interval(secs => $1)
         ORDER BY started_at",
    )
    .bind(ACTIVE_WINDOW_SECS as f64)
    .fetch_all(pool)
    .await
    .map_err(db_err)
}
