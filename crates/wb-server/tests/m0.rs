//! Foundation acceptance tests: they drive the real binary and a real MCP client.

mod common;

use common::TestEnv;

#[tokio::test(flavor = "multi_thread")]
async fn server_info_answers_in_two_sessions() {
    let env = TestEnv::new();
    env.setup();

    let a = env.session("client-a").await;
    let tools = a.tools().await;
    let names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names.len(), 4, "{names:?}");
    for name in ["server_info", "sql", "rerun"] {
        let tool = tools.iter().find(|t| t.name == name).expect(name);
        let hints = tool.annotations.as_ref().expect("annotations");
        assert_eq!(hints.read_only_hint, Some(true), "{name}");
        assert_eq!(hints.open_world_hint, Some(false), "{name}");
    }
    // `note` writes only Wise Bucket's context file, never the recordings.
    let note = tools.iter().find(|t| t.name == "note").expect("note");
    let hints = note.annotations.as_ref().expect("annotations");
    assert_eq!(
        (hints.read_only_hint, hints.destructive_hint),
        (Some(false), Some(false))
    );
    let info = a.server_info().await;
    assert_eq!(info["milestone"], "M1");
    assert_eq!(info["session"]["client"]["name"], "client-a");
    assert!(info["store"]["path"].as_str().unwrap().ends_with("store"));

    // A second window works at the same time.
    let b = env.session("client-b").await;
    assert_eq!(
        b.server_info().await["session"]["client"]["name"],
        "client-b"
    );
    b.close().await;
    a.server_info().await;
    a.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn server_starts_without_setup() {
    let env = TestEnv::new();
    let s = env.session("c").await;
    let info = s.server_info().await;
    assert!(info["roots"].as_array().unwrap().is_empty(), "{info:#}");
    let r = s.sql("SELECT count(*) AS n FROM recordings").await;
    assert_eq!(r["rows"][0]["n"], 0, "{r:#}");
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

    env.ok(&["roots", "add", "bags", &bags_s]);
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
        (c["mcap"].as_u64(), c["rosbag2"].as_u64()),
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
    assert_eq!(info["roots"][0]["candidate_recordings"], 2, "{info:#}");

    // What the engineer knows, from the terminal: a root, a folder, an unknown target.
    env.ok(&["note", "bags:", "hardware=rover-b", "site=lab"]);
    env.ok(&["note", "bags:run1", "site=field"]);
    let noted = env.json(&["note", "bags:run1"]);
    assert_eq!(
        noted["context"],
        serde_json::json!({"site": "field"}),
        "{noted:#}"
    );
    assert_eq!(noted["recordings"], 1, "{noted:#}");
    let unknown = env.run(&["note", "bags:nope.mcap", "a=b"]);
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("target_not_found"));
    let path = env.json(&["config", "path"]);
    let file = std::fs::read_to_string(path["context"].as_str().unwrap()).unwrap();
    assert!(
        file.contains("[\"bags:\"]") && file.contains("hardware = \"rover-b\""),
        "{file}"
    );

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
    env.setup();
    let d = env.json(&["doctor"]);
    assert_eq!(d["failed"], 0, "{d:#}");
    let checks = d["checks"].as_array().unwrap();
    for name in ["store", "sql"] {
        assert!(
            checks
                .iter()
                .any(|c| c["name"] == name && c["status"] == "ok"),
            "{name}: {d:#}"
        );
    }

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
    assert!(!env.run(&["doctor"]).status.success());
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
    assert_eq!(info["session"]["client"]["name"], "claude-code");
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
