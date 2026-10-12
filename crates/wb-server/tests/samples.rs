//! Public sample recordings, one test per format (see `wb_core::samples`).
//!
//! The samples are downloaded from their upstream repositories (pinned commits,
//! SHA-256 verified) into `target/tmp/samples`, never into the repository. The
//! tests need the network, so they are ignored by default:
//!
//! ```text
//! cargo test -p wisebucket --test samples -- --ignored            # every format
//! cargo test -p wisebucket --test samples ulog -- --ignored --exact
//! ```
//!
//! Readable formats are also extracted into the store and checked with SQL.

mod common;

use std::path::{Path, PathBuf};

use common::TestEnv;
use wb_core::samples;

fn cache_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("samples")
}

/// Leading bytes (per file suffix) proving a file is real data, not an LFS pointer or an HTML page.
const SIGNATURES: &[(&str, &[u8])] = &[
    ("metadata.yaml", b"rosbag2_bagfile_information:"),
    (".mcap", b"\x89MCAP0\r\n"),
    (".db3", b"SQLite format 3\0"),
    (".ulg", b"ULog\x01\x12\x35"),
    (".BIN", b"\xa3\x95"),
    (".asc", b"date "),
    (".blf", b"LOGG"),
    (".mf4", b"MDF     4."),
    (".parquet", b"PAR1"),
];

fn assert_signature(path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    let name = path.to_string_lossy();
    if name.ends_with(".tlog") {
        // Each record: 8-byte timestamp, then a MAVLink 2 (0xFD) or 1 (0xFE) frame.
        assert!(matches!(bytes.get(8), Some(0xFD | 0xFE)), "{name}");
        return;
    }
    let (suffix, magic) = SIGNATURES
        .iter()
        .find(|(s, _)| name.ends_with(s))
        .unwrap_or_else(|| panic!("no signature for {name}"));
    assert!(bytes.starts_with(magic), "{name}: bad header");
    // MCAP and Parquet repeat their magic at the end of a complete file.
    if matches!(*suffix, ".mcap" | ".parquet") {
        assert!(bytes.ends_with(magic), "{name}: bad footer");
    }
}

/// Fetches a format's samples (checking every file's signature) and returns their folder.
fn fetched(key: &str) -> std::path::PathBuf {
    let format = samples::find(key).unwrap();
    let cache = cache_dir();
    samples::fetch(format, &cache, &|msg| eprintln!("{msg}")).unwrap();
    let dir = cache.join(key);
    for f in format.files {
        assert_signature(&dir.join(f.path));
    }
    dir
}

/// Declares a format's samples as a root and checks how `roots check` counts them.
fn check(key: &str, kind: &str, count: u64) -> (TestEnv, std::path::PathBuf) {
    let dir = fetched(key);
    let env = TestEnv::new();
    env.setup();
    env.ok(&["roots", "add", "s", &dir.to_string_lossy()]);
    let check = env.json(&["roots", "check"]);
    assert_eq!(
        check["roots"][0]["candidates"][kind].as_u64(),
        Some(count),
        "{check:#}"
    );
    (env, dir)
}

/// Opens a session and waits until `done` recordings are extracted, none failed.
async fn extracted(env: &TestEnv, done: u64) -> common::Session {
    let s = env.session("samples").await;
    let n = |i: &serde_json::Value, st: &str| i["store"]["recordings"][st].as_u64().unwrap_or(0);
    s.wait_until("extraction", |i| n(i, "done") + n(i, "failed") == done)
        .await;
    let failed = s
        .sql("SELECT recording, error FROM recordings WHERE status = 'failed'")
        .await;
    assert_eq!(failed["row_count"], 0, "{failed:#}");
    s
}

async fn number(s: &common::Session, query: &str) -> u64 {
    let r = s.sql(query).await;
    r["rows"][0]["n"]
        .as_u64()
        .unwrap_or_else(|| panic!("{query}: {r:#}"))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network: fetches public samples"]
async fn ros2_mcap() {
    let (env, _) = check("ros2-mcap", "rosbag2", 1);
    let s = extracted(&env, 1).await;
    assert!(number(&s, "SELECT count(*) AS n FROM stream('/topic')").await > 0);
    let r = s
        .sql("SELECT decoded FROM streams WHERE stream = '/rosout'")
        .await;
    assert_eq!(r["rows"][0]["decoded"], true, "{r:#}");
    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network: fetches public samples"]
async fn ros2_db3() {
    let (env, _) = check("ros2-db3", "rosbag2", 1);
    let s = extracted(&env, 1).await;
    assert!(
        number(
            &s,
            "SELECT count(*) AS n FROM stream('/topic') WHERE data IS NOT NULL"
        )
        .await
            > 0
    );
    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network: fetches public samples"]
async fn mcap() {
    let (env, _) = check("mcap", "mcap", 1);
    let s = extracted(&env, 1).await;
    assert!(number(&s, "SELECT count(*) AS n FROM streams").await > 0);
    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network: fetches public samples"]
async fn ulog() {
    let (env, _) = check("ulog", "ulog", 6);
    let s = extracted(&env, 6).await;
    assert!(
        number(
            &s,
            "SELECT count(*) AS n FROM streams WHERE decoded AND rows > 0"
        )
        .await
            > 10
    );
    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network: fetches public samples"]
async fn parquet() {
    let (env, _) = check("parquet", "parquet", 1);
    let s = extracted(&env, 1).await;
    assert_eq!(
        number(
            &s,
            "SELECT count(*) AS n FROM stream('table') WHERE t IS NOT NULL"
        )
        .await,
        8
    );
    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network: fetches public samples"]
async fn tlog() {
    let (env, _) = check("tlog", "tlog", 1);
    let s = extracted(&env, 1).await;
    let r = s
        .sql(
            "SELECT CAST(t AS BIGINT) AS t, voltages[1] AS v, battery_remaining, \
             battery_function, system_id FROM stream('BATTERY_STATUS')",
        )
        .await;
    assert_eq!(r["row_count"], 1, "{r:#}");
    let row = &r["rows"][0];
    assert_eq!(row["t"], 47_333_000_000_i64, "{r:#}");
    assert_eq!(row["v"], 12587, "{r:#}");
    assert_eq!(row["battery_remaining"], 100, "{r:#}");
    assert_eq!(
        row["battery_function"], "MAV_BATTERY_FUNCTION_UNKNOWN",
        "{r:#}"
    );
    assert_eq!(row["system_id"], 1, "{r:#}");
    // The truncated tail is reported, not an error.
    let r = s.sql("SELECT notes FROM recordings").await;
    assert!(
        r["rows"][0]["notes"].as_str().unwrap().contains("skipped"),
        "{r:#}"
    );
    s.close().await;
}

// The formats below have no reader yet: `roots check` counts them as `other`.

#[test]
#[ignore = "network: fetches public samples"]
fn dataflash() {
    check("dataflash", "other", 1);
}

#[test]
#[ignore = "network: fetches public samples"]
fn can() {
    check("can", "other", 2);
}

#[test]
#[ignore = "network: fetches public samples"]
fn mdf4() {
    check("mdf4", "other", 1);
}

#[test]
fn every_format_has_a_test() {
    let tests = [
        "ros2-mcap",
        "ros2-db3",
        "mcap",
        "ulog",
        "dataflash",
        "tlog",
        "can",
        "mdf4",
        "parquet",
    ];
    assert_eq!(samples::keys().collect::<Vec<_>>(), tests);
}

/// The `demo` command end to end: download, declare the root, rerun from cache,
/// and see the root over MCP.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "network: fetches public samples"]
async fn demo_command() {
    let env = TestEnv::new();
    env.setup();
    env.ok(&["demo", "ros2-mcap", "tlog", "-y"]);
    let demo = env.state.join("demo");
    assert!(demo.join("ros2-mcap/talker/talker.mcap").is_file());
    assert!(demo.join("tlog/capture.mav2.battery_status.tlog").is_file());

    let again = env.ok(&["demo", "ros2-mcap", "tlog", "-y"]);
    let stderr = String::from_utf8_lossy(&again.stderr);
    assert!(!stderr.contains("Downloading"), "{stderr}");
    assert!(stderr.contains("already declared"), "{stderr}");

    let s = env.session("c").await;
    let info = s.server_info().await;
    assert_eq!(info["roots"][0]["name"], "demo", "{info:#}");
    s.close().await;
}
