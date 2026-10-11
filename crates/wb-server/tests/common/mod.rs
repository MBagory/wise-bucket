//! Test harness: isolated configuration and state per test.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, Implementation,
    InitializeRequestParams,
};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

pub const BIN: &str = env!("CARGO_BIN_EXE_wisebucket");

pub struct TestEnv {
    // Kept alive for the duration of the test.
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub cwd: PathBuf,
    pub extra_env: Vec<(String, String)>,
}

impl TestEnv {
    /// A fresh environment. Uses a short path under /tmp so the Unix socket path stays short.
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("wbt")
            .tempdir_in("/tmp")
            .unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let cwd = root.join("cwd");
        std::fs::create_dir_all(&cwd).unwrap();
        Self {
            config: root.join("config.toml"),
            state: root.join("state"),
            cwd,
            root,
            _dir: dir,
            extra_env: Vec::new(),
        }
    }

    pub fn env(mut self, k: &str, v: &str) -> Self {
        self.extra_env.push((k.into(), v.into()));
        self
    }

    fn apply(&self, cmd: &mut std::process::Command) {
        cmd.env("WB_CONFIG", &self.config)
            .env("WB_STATE_DIR", &self.state)
            .current_dir(&self.cwd);
        for (k, v) in &self.extra_env {
            cmd.env(k, v);
        }
    }

    /// Runs the CLI and returns its output.
    pub fn run(&self, args: &[&str]) -> Output {
        let mut cmd = std::process::Command::new(BIN);
        self.apply(&mut cmd);
        cmd.args(args).stdin(Stdio::null()).output().unwrap()
    }

    /// Runs the CLI and asserts success.
    pub fn ok(&self, args: &[&str]) -> Output {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "`{}` failed:\nstdout: {}\nstderr: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    pub fn json(&self, args: &[&str]) -> Value {
        let mut a = vec!["--json"];
        a.extend_from_slice(args);
        let out = self.run(&a);
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "invalid JSON from {args:?}: {e}\n{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        })
    }

    /// Managed setup (non-interactive).
    pub fn setup(&self) {
        self.ok(&["setup", "--yes"]);
    }

    /// The server log lines of every session (all days).
    pub fn server_log(&self) -> String {
        let mut out = String::new();
        if let Ok(entries) = std::fs::read_dir(self.state.join("logs")) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with("server.log") {
                    out.push_str(&std::fs::read_to_string(e.path()).unwrap_or_default());
                }
            }
        }
        out
    }

    /// Starts an MCP session (the server process plus an rmcp client).
    pub async fn session(&self, client_name: &str) -> Session {
        self.session_with(BIN, &["serve"], client_name).await
    }

    pub async fn session_with(&self, program: &str, args: &[&str], client_name: &str) -> Session {
        let mut cmd = tokio::process::Command::new(program);
        let mut std_cmd = std::process::Command::new(program);
        self.apply(&mut std_cmd);
        for (k, v) in std_cmd.get_envs() {
            match v {
                Some(v) => cmd.env(k, v),
                None => cmd.env_remove(k),
            };
        }
        cmd.current_dir(&self.cwd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();
        let info = InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new(client_name, "1.0.0"),
        );
        let client = info.serve((stdout, stdin)).await.expect("MCP initialize");
        Session {
            client: Some(client),
            child,
        }
    }
}

pub struct Session {
    client: Option<RunningService<RoleClient, InitializeRequestParams>>,
    child: tokio::process::Child,
}

impl Session {
    pub async fn server_info(&self) -> Value {
        let res = self
            .client
            .as_ref()
            .unwrap()
            .call_tool(CallToolRequestParams::new("server_info"))
            .await
            .expect("server_info call");
        assert_ne!(
            res.is_error,
            Some(true),
            "server_info returned an error result"
        );
        res.structured_content.expect("structured content")
    }

    /// Calls a tool with JSON arguments.
    pub async fn call(&self, name: &str, args: serde_json::Value) -> CallToolResult {
        let mut params = CallToolRequestParams::new(name.to_string());
        if let serde_json::Value::Object(map) = args {
            params = params.with_arguments(map);
        }
        self.client
            .as_ref()
            .unwrap()
            .call_tool(params)
            .await
            .unwrap_or_else(|e| panic!("{name} call: {e}"))
    }

    /// Runs a query with the `sql` tool and returns its structured result, rows as objects.
    pub async fn sql(&self, query: &str) -> Value {
        let res = self
            .call("sql", serde_json::json!({ "query": query }))
            .await;
        objects(res.structured_content.expect("structured content"))
    }

    /// Polls `server_info` until `ready` holds (each call also looks for new recordings).
    pub async fn wait_until(&self, what: &str, ready: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        loop {
            let info = self.server_info().await;
            if ready(&info) {
                return info;
            }
            assert!(
                start.elapsed() < Duration::from_secs(90),
                "timed out waiting for {what}: {info:#}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    pub async fn tools(&self) -> Vec<rmcp::model::Tool> {
        self.client
            .as_ref()
            .unwrap()
            .list_all_tools()
            .await
            .unwrap()
    }

    /// Closes stdin like a harness does and waits for a graceful exit.
    pub async fn close(mut self) {
        if let Some(c) = self.client.take() {
            let _ = c.cancel().await;
        }
        let status = tokio::time::timeout(Duration::from_secs(60), self.child.wait())
            .await
            .expect("server did not exit after stdin closed")
            .unwrap();
        assert!(status.success(), "server exited with {status}");
    }
}

pub fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// A `sql` result with its rows as objects keyed by column, for readable assertions.
pub fn objects(mut r: Value) -> Value {
    let columns = r["columns"].as_array().cloned().unwrap_or_default();
    if let Some(rows) = r["rows"].as_array_mut() {
        for row in rows {
            let cells = row.as_array().cloned().unwrap_or_default();
            *row = Value::Object(
                columns
                    .iter()
                    .map(|c| c.as_str().unwrap().to_string())
                    .zip(cells)
                    .collect(),
            );
        }
    }
    r
}
