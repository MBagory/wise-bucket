//! Milestone M1 acceptance tests: recordings are extracted into the store in the
//! background while a session is open, and the agent computes metrics with `sql`.
//! Format correctness is tested in `wb-data`; here the whole chain is exercised.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Session, TestEnv};
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

/// Polls `sql` (which, unlike `server_info`, never walks the roots) until `path` is done.
async fn done_within(s: &Session, path: &str, limit: Duration) {
    let start = Instant::now();
    loop {
        let r = s
            .sql(&format!(
                "SELECT status FROM recordings WHERE path = '{path}'"
            ))
            .await;
        if r["rows"][0]["status"] == "done" {
            return;
        }
        assert!(
            start.elapsed() < limit,
            "{path} not extracted within {limit:?}: {r:#}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn recordings_are_extracted_and_queried() {
    let env = TestEnv::new();
    env.setup();
    let bags = env.root.join("bags");
    bench(&bags.join("bench1.csv"), 10.0, 50);
    bench(&bags.join("bench2.csv"), 20.0, 50);
    common::write(&bags.join("corrupt.mcap"), "not an MCAP file");
    env.ok(&["roots", "add", "bags", &bags.to_string_lossy()]);

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
        .sql("SELECT status, error FROM recordings WHERE path = 'corrupt.mcap'")
        .await;
    assert_eq!(r["rows"][0]["status"], "failed", "{r:#}");
    assert!(
        r["rows"][0]["error"]
            .as_str()
            .unwrap()
            .contains("corrupt.mcap")
    );

    // What the engineer says goes to `recordings.context`, through the `note` tool.
    let res = s
        .call(
            "note",
            json!({"target": "bags:", "context": {"hardware": "rig-1"}}),
        )
        .await;
    let noted = res.structured_content.unwrap();
    assert_eq!(
        (&noted["target"], &noted["recordings"]),
        (&json!("bags:"), &json!(3)),
        "{noted:#}"
    );
    s.call(
        "note",
        json!({"target": "bags:bench2.csv", "context": {"hardware": "rig-2", "wind": "gusty"}}),
    )
    .await;
    let r = s
        .sql("SELECT recording, context FROM recordings ORDER BY recording")
        .await;
    assert_eq!(
        r["rows"],
        json!([
            {"recording": "bags:bench1.csv", "context": {"hardware": "rig-1"}},
            {"recording": "bags:bench2.csv", "context": {"hardware": "rig-2", "wind": "gusty"}},
            {"recording": "bags:corrupt.mcap", "context": {"hardware": "rig-1"}},
        ]),
        "{r:#}"
    );
    let res = s
        .call(
            "note",
            json!({"target": "bags:nope.csv", "context": {"x": "y"}}),
        )
        .await;
    assert_eq!(res.is_error, Some(true));
    assert_eq!(
        res.structured_content.unwrap()["error"]["code"],
        "target_not_found"
    );
    // An empty value or null removes a key.
    s.call(
        "note",
        json!({"target": "bags:bench2.csv", "context": {"hardware": null, "wind": ""}}),
    )
    .await;
    let r = s
        .sql("SELECT context['hardware'] AS hw FROM recordings WHERE recording = 'bags:bench2.csv'")
        .await;
    assert_eq!(r["rows"][0]["hw"], "rig-1", "{r:#}");

    // A file added during the session is extracted without any command.
    bench(&bags.join("bench3.csv"), 30.0, 20);
    s.wait_until("3 done", |i| count(i, "done") == 3).await;
    let r = s
        .sql("SELECT count(*) AS n FROM stream('table', 'bags:bench3%')")
        .await;
    assert_eq!(r["rows"][0]["n"], 20);

    // File events start the extraction: at once for a file renamed into place (a
    // finished download), once it has settled for a file written in place.
    bench(&bags.join("bench4.csv.partial"), 40.0, 20);
    std::fs::rename(bags.join("bench4.csv.partial"), bags.join("bench4.csv")).unwrap();
    // Under the 5 s a file written in place waits.
    done_within(&s, "bench4.csv", Duration::from_secs(4)).await;
    bench(&bags.join("bench5.csv"), 50.0, 20);
    done_within(&s, "bench5.csv", Duration::from_secs(20)).await;

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
    let res = s
        .call(
            "sql",
            json!({ "query": "SELECT * FROM generate_series(1, 500)", "limit": 2 }),
        )
        .await;
    let r = res.structured_content.unwrap();
    assert_eq!(
        (&r["rows"], &r["truncated"]),
        (&json!([[1], [2]]), &json!(true)),
        "{r:#}"
    );
    assert!(r["hint"].as_str().unwrap().contains("limit 2"), "{r:#}");
    s.close().await;

    // The same query from a terminal.
    let out = env.ok(&["--json", "sql", "SELECT count(*) AS n FROM recordings"]);
    let r = common::objects(serde_json::from_slice(&out.stdout).unwrap());
    assert_eq!(r["rows"][0]["n"], 6, "{r:#}");
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
