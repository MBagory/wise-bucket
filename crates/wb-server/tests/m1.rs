//! Milestone M1 acceptance tests: recordings are extracted into the store in the
//! background while a session is open, and the agent computes metrics with `sql`.
//! Format correctness is tested in `wb-data`; here the whole chain is exercised.

mod common;

use std::path::Path;

use common::TestEnv;
use serde_json::{Value, json};

/// A CSV recording at 10 Hz: `time_s;voltage`, voltage = `base` + i / 10.
fn bench(path: &Path, base: f64, rows: usize) {
    let mut text = String::from("time_s;voltage\n");
    for i in 0..rows {
        text.push_str(&format!("{};{}\n", i as f64 / 10.0, base + i as f64 / 10.0));
    }
    common::write(path, &text);
}

fn count(info: &Value, status: &str) -> u64 {
    info["store"]["recordings"][status].as_u64().unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn recordings_are_extracted_and_queried() {
    let env = TestEnv::new();
    env.setup();
    let bags = env.root.join("bags");
    bench(&bags.join("bench1.csv"), 10.0, 50);
    bench(&bags.join("bench2.csv"), 20.0, 50);
    common::write(&bags.join("corrupt.mcap"), "not an MCAP file");
    env.ok(&[
        "roots",
        "add",
        "bags",
        &bags.to_string_lossy(),
        "--hardware",
        "rig-1",
    ]);

    let s = env.session("c").await;
    s.wait_until("2 done and 1 failed", |i| {
        count(i, "done") == 2 && count(i, "failed") == 1
    })
    .await;
    let r = s
        .sql("SELECT recording, max(voltage) AS v, count(*) AS n FROM stream('table') GROUP BY recording ORDER BY recording")
        .await;
    assert_eq!(
        r["rows"],
        json!([
            {"recording": "bags:bench1.csv", "v": 14.9, "n": 50},
            {"recording": "bags:bench2.csv", "v": 24.9, "n": 50},
        ]),
        "{r:#}"
    );
    let r = s
        .sql("SELECT status, hardware, error FROM recordings WHERE path = 'corrupt.mcap'")
        .await;
    assert_eq!(r["rows"][0]["status"], "failed", "{r:#}");
    assert_eq!(r["rows"][0]["hardware"], "rig-1");
    assert!(
        r["rows"][0]["error"]
            .as_str()
            .unwrap()
            .contains("corrupt.mcap")
    );

    // A file added during the session is extracted without any command.
    bench(&bags.join("bench3.csv"), 30.0, 20);
    s.wait_until("3 done", |i| count(i, "done") == 3).await;
    let r = s
        .sql("SELECT count(*) AS n FROM stream('table', 'bags:bench3%')")
        .await;
    assert_eq!(r["rows"][0]["n"], 20);

    // A changed file is extracted again; a deleted one stays queryable as `missing`.
    bench(&bags.join("bench1.csv"), 100.0, 50);
    std::fs::remove_file(bags.join("bench2.csv")).unwrap();
    s.wait_until("bench2 missing", |i| count(i, "missing") == 1)
        .await;
    let mut v = Value::Null;
    for _ in 0..180 {
        let r = s
            .sql("SELECT max(voltage) AS v FROM stream('table', 'bags:bench1%')")
            .await;
        v = r["rows"][0]["v"].clone();
        if v == json!(104.9) {
            break;
        }
        s.server_info().await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert_eq!(v, json!(104.9), "bench1.csv was not extracted again");
    let r = s
        .sql("SELECT count(*) AS n FROM stream('table', 'bags:bench2%')")
        .await;
    assert_eq!(r["rows"][0]["n"], 50);

    // Errors carry a code, a fix and a link; writes are refused; results are capped.
    for bad in ["DROP TABLE recordings", "SELECT * FROM nowhere"] {
        let res = s.call("sql", json!({ "query": bad })).await;
        assert_eq!(res.is_error, Some(true), "{bad}");
        let e = &res.structured_content.unwrap()["error"];
        assert_eq!(e["code"], "query_failed", "{bad}");
        assert!(e["docs_url"].as_str().unwrap().ends_with("#query_failed"));
    }
    let r = s.sql("SELECT * FROM generate_series(1, 500)").await;
    assert_eq!(
        (r["row_count"].as_u64(), &r["truncated"]),
        (Some(200), &json!(true))
    );
    s.close().await;

    // The same query from a terminal.
    let out = env.ok(&["--json", "sql", "SELECT count(*) AS n FROM recordings"]);
    let r: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(r["rows"][0]["n"], 4, "{r:#}");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_sessions_extract_each_file_once() {
    let env = TestEnv::new();
    env.setup();
    let bags = env.root.join("bags");
    let names: Vec<String> = (0..6).map(|i| format!("run{i}.csv")).collect();
    for (i, n) in names.iter().enumerate() {
        bench(&bags.join(n), i as f64, 30);
    }
    env.ok(&["roots", "add", "bags", &bags.to_string_lossy()]);

    let a = env.session("a").await;
    let b = env.session("b").await;
    let done = |i: &Value| count(i, "done") == names.len() as u64;
    a.wait_until("all done", done).await;
    let r = b.sql("SELECT count(*) AS n FROM stream('table')").await;
    assert_eq!(r["rows"][0]["n"], 30 * names.len());
    a.close().await;
    b.close().await;

    let log = env.server_log();
    for n in &names {
        let extracted = log
            .lines()
            .filter(|l| l.contains(&format!("bags:{n}")) && l.contains("stream(s) extracted"))
            .count();
        assert_eq!(extracted, 1, "{n} extracted {extracted} times:\n{log}");
    }
}
