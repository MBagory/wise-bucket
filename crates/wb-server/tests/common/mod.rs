//! Test harness: isolated configuration + state per test, shared PostgreSQL runtime.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, Implementation, InitializeRequestParams,
};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

pub const BIN: &str = env!("CARGO_BIN_EXE_wisebucket");

/// Shared runtime (PostgreSQL + pgvector) so each test does not rebuild it.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("WB_TEST_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wb runtime"))
}

pub struct TestEnv {
    // Kept alive for the duration of the test.
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub repo: PathBuf,
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
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        Self {
            config: root.join("config.toml"),
            state: root.join("state"),
            repo,
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
        cmd.env_remove("WB_PROJECT_DIR")
            .env_remove("WB_KEEP_RUNNING")
            .env("WB_CONFIG", &self.config)
            .env("WB_STATE_DIR", &self.state)
            .env("WB_RUNTIME_DIR", runtime_dir())
            .current_dir(&self.repo);
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

    pub fn pg_running(&self) -> bool {
        let st = self.json(&["db", "status"]);
        st["running"].as_bool().unwrap_or(false)
    }

    pub fn wait_pg(&self, running: bool, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.pg_running() == running {
                return true;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        false
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
        cmd.current_dir(&self.repo)
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

    pub async fn tool_names(&self) -> Vec<String> {
        self.client
            .as_ref()
            .unwrap()
            .list_all_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect()
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
