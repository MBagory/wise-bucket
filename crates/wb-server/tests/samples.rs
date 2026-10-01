//! Public sample recordings, one test per format (see `wb_core::samples`).
//!
//! The samples are downloaded from their upstream repositories (pinned commits,
//! SHA-256 verified) into `target/tmp/samples`, never into the repository. The
//! tests need the network, so they are ignored by default:
//!
//! ```text
//! cargo test -p wb-server --test samples -- --ignored            # every format
//! cargo test -p wb-server --test samples ulog -- --ignored --exact
//! ```
//!
//! From M3 on, each format test grows a decode check against its manifest.

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
    (".bag", b"#ROSBAG V2.0\n"),
    (".ulg", b"ULog\x01\x12\x35"),
    (".BIN", b"\xa3\x95"),
    (".asc", b"date "),
    (".blf", b"LOGG"),
    (".dbc", b"VERSION"),
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

/// Fetches a format, checks every file's signature, then declares it as a root
/// and checks how `roots check` counts it.
fn check(key: &str, kind: &str, count: u64) {
    let format = samples::find(key).unwrap();
    let cache = cache_dir();
    samples::fetch(format, &cache, &|msg| eprintln!("{msg}")).unwrap();
    let dir = cache.join(key);
    for f in format.files {
        assert_signature(&dir.join(f.path));
    }

    let env = TestEnv::new();
    env.ok(&["roots", "add", "s", &dir.to_string_lossy()]);
    let check = env.json(&["roots", "check"]);
    assert_eq!(
        check["roots"][0]["candidates"][kind].as_u64(),
        Some(count),
        "{check:#}"
    );
}

#[test]
#[ignore = "network: fetches public samples"]
fn ros2_mcap() {
    check("ros2-mcap", "rosbag2_dirs", 1);
}

#[test]
#[ignore = "network: fetches public samples"]
fn ros2_db3() {
    check("ros2-db3", "rosbag2_dirs", 1);
}

#[test]
#[ignore = "network: fetches public samples"]
fn mcap() {
    check("mcap", "mcap", 2);
}

#[test]
#[ignore = "network: fetches public samples"]
fn ros1_bag() {
    check("ros1-bag", "ros1_bag", 4);
}

#[test]
#[ignore = "network: fetches public samples"]
fn ulog() {
    check("ulog", "ulog", 1);
}

// The formats below have no reader yet: `roots check` counts them as `other`.

#[test]
#[ignore = "network: fetches public samples"]
fn dataflash() {
    check("dataflash", "other", 1);
}

#[test]
#[ignore = "network: fetches public samples"]
fn tlog() {
    check("tlog", "other", 1);
}

#[test]
#[ignore = "network: fetches public samples"]
fn can() {
    check("can", "other", 3);
}

#[test]
#[ignore = "network: fetches public samples"]
fn mdf4() {
    check("mdf4", "other", 1);
}

#[test]
#[ignore = "network: fetches public samples"]
fn parquet() {
    check("parquet", "other", 1);
}

#[test]
fn every_format_has_a_test() {
    let tests = [
        "ros2-mcap",
        "ros2-db3",
        "mcap",
        "ros1-bag",
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
