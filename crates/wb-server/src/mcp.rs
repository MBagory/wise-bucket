//! The MCP server (stdio).

use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, InitializeRequestParams, InitializeResult,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use wb_core::config::{self, EffectiveConfig};
use wb_core::error::Result;
use wb_core::roots::{self, CandidateCounts, RootSet};
use wb_data::store::{self, Store};

use crate::ui;
use crate::worker::Worker;

/// Instructions sent to the harness in `initialize`.
pub const INSTRUCTIONS: &str = include_str!("instructions.md");

/// Shared server state.
pub struct AppState {
    pub cfg: EffectiveConfig,
    /// Kept so data roots can be re-read on each call: `roots add` and `demo`
    /// take effect without restarting the harness.
    pub overrides: config::Overrides,
    pub client: Mutex<Option<(String, String)>>,
    pub worker: Option<Worker>,
}

#[derive(Clone)]
pub struct WbServer {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

/// Arguments of the `sql` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SqlArgs {
    /// One read-only SELECT query.
    pub query: String,
    /// Rows wanted at most (the result is also capped at about 60 KB).
    pub limit: Option<usize>,
}

/// Arguments of the `rerun` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RerunArgs {
    /// One read-only SELECT query with a timestamp column `t`.
    pub query: String,
    /// Entity path prefix: series are named `<entity>/<column>` (e.g. the stream name).
    pub entity: Option<String>,
    /// Address of the Rerun viewer (default `rerun+http://127.0.0.1:9876/proxy`).
    pub url: Option<String>,
}

fn sql_description() -> String {
    format!(
        "Run one read-only SQL query (DataFusion dialect, close to PostgreSQL) over the store: a decoded, columnar copy of every recording in the data roots. Compute any metric with it; nothing is precomputed. Rows are arrays in the order of `columns`. Every row enters your context: aggregate or filter before fetching raw rows. A result is capped at {} KB and at `limit` rows if you set one; `truncated` and `hint` say when more exist and how to get them. Page by time window or by key (recipes), not with OFFSET: the store changes while you page. {} s per query; only SELECT is allowed. Quote dotted or upper-case column names (\"linear_acceleration.z\"). Times are nanosecond timestamps.\n\nTables:\n{}\nRecipes:\n{}",
        wb_data::sql::MAX_BYTES / 1000,
        wb_data::sql::TIMEOUT.as_secs(),
        wb_data::tables::summary(),
        wb_data::tables::RECIPES
    )
}

impl WbServer {
    pub fn new(state: Arc<AppState>) -> Self {
        let mut tool_router = Self::tool_router();
        if let Some(route) = tool_router.map.get_mut("sql") {
            route.attr.description = Some(sql_description().into());
        }
        Self { state, tool_router }
    }

    /// The configuration as it is now (the files may have changed since startup).
    fn current_config(&self) -> std::result::Result<EffectiveConfig, wb_core::WbError> {
        config::load(&self.state.overrides)
    }

    async fn server_info_value(&self) -> Value {
        let s = &self.state;
        let cfg = self.current_config();
        let root_set = match &cfg {
            Ok(cfg) => roots::resolve(cfg),
            Err(e) => RootSet {
                roots: Vec::new(),
                problems: vec![(None, wb_core::error::err(e.kind(), e.message()))],
            },
        };
        let cfg = cfg.unwrap_or_else(|_| s.cfg.clone());
        // Walk the roots now, so a file added a moment ago is counted and extracted soon.
        // ponytail: walks every root on each call (capped per root); rely on the
        // background discovery alone if large roots make this slow.
        let walk_cfg = cfg.clone();
        let entries = tokio::task::spawn_blocking(move || {
            store::discover(&walk_cfg, Duration::from_secs(5)).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        if let Some(w) = &s.worker {
            w.wake();
        }
        let store = Store::for_config(&cfg);
        let counts = store.counts();
        let roots: Vec<Value> = root_set
            .roots
            .iter()
            .map(|r| {
                let mut c = CandidateCounts::default();
                for e in entries.iter().filter(|e| e.candidate.root == r.name) {
                    *c.by_kind.entry(e.candidate.kind).or_default() += 1;
                }
                json!({
                    "name": r.name,
                    "path": r.path,
                    "hardware": r.hardware,
                    "declared_in": r.origin.to_string(),
                    "candidate_recordings": c.total(),
                    "candidates": c,
                })
            })
            .collect();
        let root_problems = crate::commands::roots::problems_json(&root_set);
        let client = s.client.lock().await.clone();
        json!({
            "wise_bucket_version": wb_core::VERSION,
            "milestone": "M1",
            "session": {
                "client": client.map(|(n, v)| json!({ "name": n, "version": v })),
            },
            "state_dir": cfg.state_dir.value,
            "store": {
                "path": store.dir,
                "size_bytes": store.size(),
                "recordings": counts,
                "paused": store.paused(),
            },
            "roots": roots,
            "root_problems": root_problems,
            "docs": ui::DOCS_BASE,
        })
    }
}

fn summary(v: &Value) -> String {
    let st = &v["store"];
    let counts = st["recordings"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, n)| format!("{n} {k}"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "empty".into());
    let store_line = format!(
        "store: {counts} · {} · {}{}",
        ui::bytes(st["size_bytes"].as_u64().unwrap_or(0)),
        st["path"].as_str().unwrap_or("?"),
        if st["paused"] == true {
            " · extraction PAUSED: little free disk space"
        } else {
            ""
        }
    );
    let roots = v["roots"].as_array().cloned().unwrap_or_default();
    let roots_line = if roots.is_empty() {
        "data roots: none declared (the engineer adds them with `wisebucket roots add <name> <path>`)".to_string()
    } else {
        format!(
            "data roots: {}",
            roots
                .iter()
                .map(|r| {
                    format!(
                        "{} → {}{} ({} recordings)",
                        r["name"].as_str().unwrap_or("?"),
                        r["path"].as_str().unwrap_or("?"),
                        r["hardware"]
                            .as_str()
                            .map(|h| format!(", hardware {h}"))
                            .unwrap_or_default(),
                        r["candidate_recordings"]
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    let problems = v["root_problems"].as_array().map(|a| a.len()).unwrap_or(0);
    format!(
        "Wise Bucket {} (milestone M1)\n{store_line}\n{roots_line}{}",
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
    /// Reports installation state: version, store and data roots.
    #[tool(
        name = "server_info",
        description = "Report Wise Bucket's state: version, the store (how many recordings are extracted, pending or failed, its size) and the data roots (the only folders Wise Bucket may read) with a count of recordings in each. Also looks for new recordings. Errors include a code, a fix and a documentation link.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn server_info(&self) -> std::result::Result<CallToolResult, McpError> {
        let v = self.server_info_value().await;
        let text = summary(&v);
        let mut res = CallToolResult::structured(v);
        res.content = vec![ContentBlock::text(text)];
        Ok(res)
    }

    /// Runs one read-only SQL query over the store (description set in `new`).
    #[tool(
        name = "sql",
        description = "Run one read-only SQL query over the store.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn sql(
        &self,
        Parameters(args): Parameters<SqlArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let result = match self.current_config() {
            Ok(cfg) => wb_data::sql::query(&Store::for_config(&cfg), &args.query, args.limit).await,
            Err(e) => Err(e),
        };
        Ok(match result {
            Ok(r) => CallToolResult::structured(json!(r)),
            Err(e) => CallToolResult::structured_error(json!({ "error": e.to_json() })),
        })
    }

    /// Sends the rows of a query to a running Rerun viewer.
    #[tool(
        name = "rerun",
        description = "Send every row of one read-only SELECT (same tables as `sql`) to a running Rerun viewer, with no size cap: the rows go to the viewer, not to your context, so send raw streams rather than bins. The query must return a timestamp column `t`. Each timestamp column (`t`, `t_sensor`…) becomes a Rerun timeline (times before 2000 count from boot and are shown as durations); each numeric or boolean column a scalar series named `<entity>/<column>`: name series with SQL aliases (AS \"imu/az\") and set `entity` to the stream name to keep streams apart. Other columns are skipped and listed. Every call adds to the same Rerun recording, `wisebucket`; move its time cursor with the Rerun viewer's MCP tools on the timeline `t`.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn rerun(
        &self,
        Parameters(args): Parameters<RerunArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let url = args.url.as_deref().unwrap_or(wb_data::rerun::DEFAULT_URL);
        let result = match self.current_config() {
            Ok(cfg) => {
                wb_data::rerun::send(
                    &Store::for_config(&cfg),
                    &args.query,
                    args.entity.as_deref(),
                    url,
                )
                .await
            }
            Err(e) => Err(e),
        };
        Ok(match result {
            Ok(r) => CallToolResult::structured(json!(r)),
            Err(e) => CallToolResult::structured_error(json!({ "error": e.to_json() })),
        })
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
        *self.state.client.lock().await = Some((name, version));
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

    let state = Arc::new(AppState {
        cfg,
        overrides: overrides.clone(),
        client: Mutex::new(None),
        worker: Some(Worker::spawn(overrides.clone())),
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
    tracing::info!("client disconnected; shutting down");
    if let Some(w) = &state.worker {
        w.stop();
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
