//! The MCP server (stdio).

use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, InitializeRequestParams, InitializeResult,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use wb_core::config::{self, EffectiveConfig};
use wb_core::db::{self, Db, Purpose};
use wb_core::error::{Result, WbError};
use wb_core::roots::{self, RootSet};
use wb_core::session::{self, SessionId, SessionStart};

use crate::ui;

/// Instructions sent to the harness in `initialize`.
pub const INSTRUCTIONS: &str = include_str!("instructions.md");

/// Shared server state.
pub struct AppState {
    pub cfg: EffectiveConfig,
    /// Kept so data roots can be re-read on each call: `roots add` and `demo`
    /// take effect without restarting the harness.
    pub overrides: config::Overrides,
    /// The database, or why it is unavailable (the server still starts so the
    /// user can be told how to fix it from inside the harness).
    pub db: std::result::Result<Db, WbError>,
    pub session_id: Option<SessionId>,
    pub client: Mutex<Option<(String, String)>>,
}

#[derive(Clone)]
pub struct WbServer {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

impl WbServer {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    /// The data roots as declared now (the config file may have changed since startup).
    fn current_roots(&self) -> RootSet {
        match config::load(&self.state.overrides) {
            Ok(cfg) => roots::resolve(&cfg),
            Err(e) => RootSet {
                roots: Vec::new(),
                problems: vec![(None, e)],
            },
        }
    }

    async fn server_info_value(&self) -> Value {
        let s = &self.state;
        let root_set = self.current_roots();
        let database = match &s.db {
            Ok(db) => {
                let active = session::active(&db.pool).await.map(|v| v.len()).ok();
                json!({
                    "status": "ok",
                    "postgresql": db.info.server_version,
                    "pgvector": db.info.vector_version,
                    "pg_trgm": db.info.pg_trgm_version,
                    "active_sessions": active,
                })
            }
            Err(e) => json!({ "status": "error", "error": e.to_json() }),
        };
        let roots: Vec<Value> = root_set
            .roots
            .iter()
            .map(|r| {
                json!({
                    "name": r.name,
                    "path": r.path,
                    "robot": r.robot,
                    "declared_in": r.origin.to_string(),
                })
            })
            .collect();
        let root_problems = crate::commands::roots::problems_json(&root_set);
        let client = s.client.lock().await.clone();
        json!({
            "wise_bucket_version": wb_core::VERSION,
            "milestone": "M0",
            "session": {
                "id": s.session_id,
                "client": client.map(|(n, v)| json!({ "name": n, "version": v })),
            },
            "database": database,
            "state_dir": s.cfg.state_dir.value,
            "roots": roots,
            "root_problems": root_problems,
            "docs": ui::DOCS_BASE,
        })
    }
}

fn summary(v: &Value) -> String {
    let db = &v["database"];
    let db_line = if db["status"] == "ok" {
        format!(
            "database: PostgreSQL {} · pgvector {} · {} active session(s)",
            db["postgresql"].as_str().unwrap_or("?"),
            db["pgvector"].as_str().unwrap_or("?"),
            db["active_sessions"]
        )
    } else {
        format!(
            "database: ERROR [{}] {} → fix: {} ({})",
            db["error"]["code"].as_str().unwrap_or("?"),
            db["error"]["message"].as_str().unwrap_or(""),
            db["error"]["fix"].as_str().unwrap_or(""),
            db["error"]["docs_url"].as_str().unwrap_or("")
        )
    };
    let roots = v["roots"].as_array().cloned().unwrap_or_default();
    let roots_line = if roots.is_empty() {
        "data roots: none declared (the engineer adds them with `wisebucket roots add <name> <path>`)".to_string()
    } else {
        format!(
            "data roots: {}",
            roots
                .iter()
                .map(|r| format!(
                    "{} → {}",
                    r["name"].as_str().unwrap_or("?"),
                    r["path"].as_str().unwrap_or("?")
                ))
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    let problems = v["root_problems"].as_array().map(|a| a.len()).unwrap_or(0);
    format!(
        "Wise Bucket {} (milestone M0)\n{db_line}\n{roots_line}{}",
        v["wise_bucket_version"].as_str().unwrap_or("?"),
        if problems > 0 {
            format!("\nroot problems: {problems} (see root_problems)")
        } else {
            String::new()
        }
    )
}

#[tool_router]
impl WbServer {
    /// Reports installation state: versions, database and data roots.
    #[tool(
        name = "server_info",
        description = "Report Wise Bucket's installation state: version, database (PostgreSQL + pgvector) status, this session, and the data roots (the only folders Wise Bucket may read). Errors include a code, a fix and a documentation link.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn server_info(&self) -> std::result::Result<CallToolResult, McpError> {
        let v = self.server_info_value().await;
        let text = summary(&v);
        let mut res = CallToolResult::structured(v);
        res.content = vec![ContentBlock::text(text)];
        Ok(res)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for WbServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("wise-bucket", wb_core::VERSION).with_title("Wise Bucket"),
            )
            .with_instructions(INSTRUCTIONS)
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<InitializeResult, McpError> {
        let name = request.client_info.name.clone();
        let version = request.client_info.version.clone();
        tracing::info!(client = %name, version = %version, "MCP client initialized");
        *self.state.client.lock().await = Some((name.clone(), version.clone()));
        if let (Ok(db), Some(id)) = (&self.state.db, self.state.session_id)
            && let Err(e) = session::set_client(&db.pool, id, &name, &version).await
        {
            tracing::warn!(error = %e, "cannot record MCP client");
        }
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }
}

/// Runs the MCP server until the client disconnects or the process is interrupted.
pub async fn serve(overrides: &config::Overrides) -> Result<()> {
    let cfg = config::load(overrides)?;
    let _log_guard = crate::logging::init_file(&cfg.state_dir.value.join("logs"));
    tracing::info!(version = wb_core::VERSION, state_dir = %cfg.state_dir.value.display(), "starting MCP server");

    for (name, e) in &roots::resolve(&cfg).problems {
        tracing::warn!(root = ?name, error = %e, "invalid data root");
    }

    let db = db::open(&cfg, Purpose::Session).await;
    if let Err(e) = &db {
        tracing::error!(error = %e, "database unavailable; server_info will report it");
    }
    let session_id = match &db {
        Ok(db) => {
            let start = SessionStart {
                pid: std::process::id(),
                hostname: std::env::var("HOSTNAME").ok(),
                cwd: std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string()),
            };
            match session::start(&db.pool, &start).await {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::error!(error = %e, "cannot record session");
                    None
                }
            }
        }
        Err(_) => None,
    };

    let state = Arc::new(AppState {
        cfg,
        overrides: overrides.clone(),
        db,
        session_id,
        client: Mutex::new(None),
    });

    // Heartbeat: keeps `last_seen_at` fresh for `db status` and provenance.
    let hb_state = state.clone();
    let heartbeat = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(session::HEARTBEAT_SECS));
        loop {
            tick.tick().await;
            if let (Ok(db), Some(id)) = (&hb_state.db, hb_state.session_id)
                && let Err(e) = session::heartbeat(&db.pool, id).await
            {
                tracing::warn!(error = %e, "heartbeat failed");
            }
        }
    });

    let server = WbServer::new(state.clone());
    let running = server.serve(rmcp::transport::stdio()).await.map_err(|e| {
        wb_core::error::err(
            wb_core::ErrorKind::Internal,
            format!("MCP initialization failed: {e}"),
        )
    });
    match running {
        Ok(running) => {
            tokio::select! {
                r = running.waiting() => {
                    if let Err(e) = r { tracing::warn!(error = %e, "MCP service ended with an error"); }
                }
                _ = shutdown_signal() => tracing::info!("interrupted"),
            }
        }
        Err(e) => tracing::error!(error = %e, "MCP service failed to start"),
    }
    heartbeat.abort();
    let _ = heartbeat.await;
    tracing::info!("client disconnected; shutting down");

    // Release: end the session, close the pool, stop PostgreSQL if this was the last session.
    if let Ok(db) = &state.db {
        if let Some(id) = state.session_id {
            let _ = session::end(&db.pool, id).await;
        }
        match db.release(&state.cfg).await {
            Ok(outcome) => tracing::info!(?outcome, "database released"),
            Err(e) => tracing::warn!(error = %e, "database release failed"),
        }
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return std::future::pending().await,
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
