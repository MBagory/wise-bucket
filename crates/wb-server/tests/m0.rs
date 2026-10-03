//! Milestone M0 acceptance tests: they drive the real binary and a real MCP client.
//!
//! The first run downloads PostgreSQL and builds pgvector into a shared runtime
//! directory (`target/tmp/wb-runtime`, or `WB_TEST_RUNTIME_DIR`).

mod common;

use std::time::Duration;

use common::TestEnv;

#[tokio::test(flavor = "multi_thread")]
async fn server_info_two_sessions_and_stop_after_last() {
    let env = TestEnv::new();
    env.setup();
    assert!(!env.pg_running(), "setup should leave PostgreSQL stopped");

    let a = env.session("client-a").await;
    let tools = a.tools().await;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "server_info");
    let hints = tools[0].annotations.as_ref().expect("annotations");
    assert_eq!(hints.read_only_hint, Some(true));
    let info = a.server_info().await;
    assert_eq!(info["database"]["status"], "ok", "{info:#}");
    let pgvector = info["database"]["pgvector"].as_str().unwrap();
    assert!(pgvector.starts_with("0.8"), "pgvector {pgvector}");
    assert!(info["session"]["id"].is_string());
    assert_eq!(info["session"]["client"]["name"], "client-a");

    let b = env.session("client-b").await;
    let info_b = b.server_info().await;
    assert_eq!(info_b["database"]["active_sessions"], 2, "{info_b:#}");

    b.close().await;
    assert!(
        env.pg_running(),
        "PostgreSQL must keep running while a session is open"
    );
    assert_eq!(a.server_info().await["database"]["active_sessions"], 1);

    a.close().await;
    assert!(
        env.wait_pg(false, Duration::from_secs(20)),
        "PostgreSQL should stop after the last session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn keep_running_leaves_postgres_up() {
    let env = TestEnv::new().env("WB_KEEP_RUNNING", "true");
    env.setup();
    let s = env.session("c").await;
    s.server_info().await;
    s.close().await;
    assert!(env.pg_running());
    env.ok(&["db", "stop"]);
    assert!(!env.pg_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn server_starts_without_setup_and_explains_how_to_fix() {
    let env = TestEnv::new();
    let s = env.session("c").await;
    let info = s.server_info().await;
    assert_eq!(info["database"]["status"], "error");
    assert_eq!(info["database"]["error"]["code"], "db_not_initialized");
    assert!(
        info["database"]["error"]["docs_ref"]
            .as_str()
            .unwrap()
            .ends_with("#db_not_initialized")
    );
    assert!(
        info["database"]["error"]["docs_url"]
            .as_str()
            .unwrap()
            .starts_with("https://")
    );
    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn roots_add_check_remove_and_visible_in_server_info() {
    let env = TestEnv::new();
    env.setup();
    let bags = env.root.join("bags");
    std::fs::create_dir_all(bags.join("run1")).unwrap();
    common::write(&bags.join("run1/metadata.yaml"), "");
    common::write(&bags.join("run1/run1_0.mcap"), "");
    common::write(&bags.join("loose.mcap"), "");
    let bags_s = bags.to_string_lossy().to_string();

    env.ok(&["roots", "add", "bags", &bags_s, "--robot", "rover-b"]);
    // Duplicate name and overlapping folder are refused.
    let dup = env.run(&["roots", "add", "bags", &bags_s]);
    assert!(!dup.status.success());
    assert!(String::from_utf8_lossy(&dup.stderr).contains("root_duplicate_name"));
    let inner = bags.join("run1").to_string_lossy().to_string();
    let overlap = env.run(&["roots", "add", "inner", &inner]);
    assert!(String::from_utf8_lossy(&overlap.stderr).contains("root_overlap"));
    let bad = env.run(&["roots", "add", "Bad Name", &bags_s]);
    assert!(String::from_utf8_lossy(&bad.stderr).contains("root_invalid"));

    let check = env.json(&["roots", "check"]);
    let c = &check["roots"][0]["candidates"];
    assert_eq!(
        (c["mcap"].as_u64(), c["rosbag2_dirs"].as_u64()),
        (Some(1), Some(1)),
        "{check:#}"
    );

    let s = env.session("c").await;
    let info = s.server_info().await;
    let names: Vec<_> = info["roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["bags"]);
    assert_eq!(info["roots"][0]["robot"], "rover-b");

    // A running session sees root changes without a restart.
    env.ok(&["roots", "remove", "bags"]);
    assert!(
        s.server_info().await["roots"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    s.close().await;

    assert!(
        bags.join("loose.mcap").exists(),
        "removing a root never touches the folder"
    );
    let list = env.json(&["roots", "list"]);
    assert!(list["roots"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn doctor_reports_problems_with_codes() {
    let env = TestEnv::new();
    // Before setup: the database check fails with a documented code.
    let d = env.json(&["doctor"]);
    assert!(d["failed"].as_u64().unwrap() >= 1, "{d:#}");
    assert!(
        d["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["code"] == "db_not_initialized"),
        "{d:#}"
    );

    env.setup();
    let gone = env.root.join("gone");
    std::fs::create_dir_all(&gone).unwrap();
    env.ok(&["roots", "add", "gone", &gone.to_string_lossy()]);
    std::fs::remove_dir(&gone).unwrap();
    let d = env.json(&["doctor"]);
    let checks = d["checks"].as_array().unwrap();
    assert!(
        checks
            .iter()
            .any(|c| c["code"] == "root_invalid" && c["status"] == "fail"),
        "{d:#}"
    );
    assert!(
        checks
            .iter()
            .any(|c| c["name"] == "database" && c["status"] == "ok"),
        "{d:#}"
    );
    assert!(!env.pg_running(), "doctor stops the server it started");
    assert!(!env.run(&["doctor"]).status.success());
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_and_restore_round_trip() {
    let env = TestEnv::new();
    env.setup();
    let s = env.session("before-backup").await;
    s.server_info().await;
    s.close().await;

    let file = env.root.join("wb.dump");
    env.ok(&["backup", &file.to_string_lossy()]);
    assert!(file.metadata().unwrap().len() > 0);
    assert!(
        !env.run(&["restore", &file.to_string_lossy()])
            .status
            .success(),
        "restore requires --yes"
    );

    // A session created after the backup disappears after restore.
    let s = env.session("after-backup").await;
    s.close().await;
    env.ok(&["restore", &file.to_string_lossy(), "--yes"]);
    env.ok(&["db", "start"]);
    let st = env.json(&["db", "status"]);
    assert_eq!(st["running"], true, "{st:#}");
    env.ok(&["db", "stop"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn docs_connect_your_harness() {
    // Executable twin of docs/guide.md, sections "Set up" and "Connect your agent".
    let env = TestEnv::new();
    let out = env.ok(&["setup", "--yes"]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let (claude, snippet) = stdout.split_once('\n').unwrap();
    assert!(
        claude.starts_with("claude mcp add --scope user wise-bucket -- "),
        "{claude}"
    );
    let mcp: serde_json::Value = serde_json::from_str(snippet).unwrap();
    let entry = &mcp["mcpServers"]["wise-bucket"];
    let program = entry["command"].as_str().unwrap().to_string();
    let args: Vec<String> = entry["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    assert_eq!(args, vec!["serve"]);

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let s = env.session_with(&program, &arg_refs, "claude-code").await;
    let info = s.server_info().await;
    assert_eq!(info["database"]["status"], "ok");
    s.close().await;

    let d = env.json(&["doctor"]);
    assert_eq!(d["failed"], 0, "{d:#}");
}

#[test]
fn generated_docs_are_up_to_date() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::process::Command::new(common::BIN)
        .current_dir(&root)
        .args(["docs", "gen", "--check"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "run `wisebucket docs gen`: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
