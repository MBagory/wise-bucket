//! Extraction of every supported format into the store, and SQL over it, on fixtures
//! generated here with exact expected values.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use datafusion::arrow::array::{Float64Array, RecordBatch};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use serde_json::{Value, json};
use wb_core::ErrorKind;
use wb_core::config::Origin;
use wb_core::roots::{self, Candidate, Root};
use wb_data::sql;
use wb_data::store::{self, Outcome, Status, Store};

/// 2023-11-14T22:13:20Z: a round number of seconds, so 1 s windows align with it.
const T0: i64 = 1_700_000_000_000_000_000;
const MS: i64 = 1_000_000;

const SAMPLE_DEF: &str = "std_msgs/Header header
float64 x
================================================================================
MSG: std_msgs/Header
builtin_interfaces/Time stamp
string frame_id
================================================================================
MSG: builtin_interfaces/Time
int32 sec
uint32 nanosec";

/// CDR (little endian) of a message starting with a `std_msgs/Header`, followed by `f64`s.
fn cdr(stamp_ns: i64, frame: &str, values: &[f64]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(((stamp_ns / 1_000_000_000) as i32).to_le_bytes());
    body.extend(((stamp_ns % 1_000_000_000) as u32).to_le_bytes());
    body.extend(((frame.len() + 1) as u32).to_le_bytes());
    body.extend(frame.as_bytes());
    body.push(0);
    for v in values {
        while body.len() % 8 != 0 {
            body.push(0);
        }
        body.extend(v.to_le_bytes());
    }
    let mut out = vec![0x00, 0x01, 0x00, 0x00];
    out.extend(body);
    out
}

const LOG_DEF: &str = "builtin_interfaces/Time stamp
uint8 level
string name
string msg
string file
string function
uint32 line
================================================================================
MSG: builtin_interfaces/Time
int32 sec
uint32 nanosec";

/// CDR (little endian) of an `rcl_interfaces/msg/Log`.
fn log_cdr(stamp_ns: i64, level: u8, msg: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(((stamp_ns / 1_000_000_000) as i32).to_le_bytes());
    body.extend(((stamp_ns % 1_000_000_000) as u32).to_le_bytes());
    body.push(level);
    for s in ["talker", msg, "talker.cpp", "main"] {
        while body.len() % 4 != 0 {
            body.push(0);
        }
        body.extend(((s.len() + 1) as u32).to_le_bytes());
        body.extend(s.as_bytes());
        body.push(0);
    }
    while body.len() % 4 != 0 {
        body.push(0);
    }
    body.extend(42u32.to_le_bytes());
    let mut out = vec![0x00, 0x01, 0x00, 0x00];
    out.extend(body);
    out
}

/// 10 Hz for 10 s with messages 31–49 missing: a 2 s gap after t = 3.0 s. `x` = index + offset.
fn sample_messages(offset: f64) -> Vec<(i64, Vec<u8>)> {
    (0..100)
        .filter(|i| !(31..=49).contains(i))
        .map(|i| {
            let stamp = T0 + i * 100 * MS;
            (stamp + 5 * MS, cdr(stamp, "base", &[i as f64 + offset]))
        })
        .collect()
}

fn write_mcap(path: &Path, offset: f64) {
    let mut w = mcap::Writer::new(std::io::BufWriter::new(
        std::fs::File::create(path).unwrap(),
    ))
    .unwrap();
    let sample = w
        .add_schema("my_msgs/msg/Sample", "ros2msg", SAMPLE_DEF.as_bytes())
        .unwrap();
    let image = w
        .add_schema("sensor_msgs/msg/Image", "ros2msg", b"uint8[] data")
        .unwrap();
    let qos = "- history: 1\n  depth: 10\n  reliability: 2\n  durability: 2\n  deadline:\n    sec: 0\n    nsec: 0";
    let ch = w
        .add_channel(
            sample,
            "/sample",
            "cdr",
            &BTreeMap::from([("offered_qos_profiles".into(), qos.into())]),
        )
        .unwrap();
    let cam = w
        .add_channel(image, "/camera/image", "cdr", &BTreeMap::new())
        .unwrap();
    let log = w
        .add_schema("rcl_interfaces/msg/Log", "ros2msg", LOG_DEF.as_bytes())
        .unwrap();
    let rosout = w
        .add_channel(log, "/rosout", "cdr", &BTreeMap::new())
        .unwrap();
    w.write_metadata(&mcap::records::Metadata {
        name: "rig".into(),
        metadata: BTreeMap::from([("serial".into(), "R-17".into())]),
    })
    .unwrap();
    // The bag summary repeats `recordings` and `streams`: left out.
    w.write_metadata(&mcap::records::Metadata {
        name: "rosbag2".into(),
        metadata: BTreeMap::from([("serialized_metadata".into(), "version: 9".into())]),
    })
    .unwrap();
    let mut msgs: Vec<(i64, u16, Vec<u8>)> = sample_messages(offset)
        .into_iter()
        .map(|(t, d)| (t, ch, d))
        .collect();
    msgs.extend((0..20).map(|i| (T0 + i * 500 * MS, cam, vec![0u8; 1000])));
    msgs.push((
        T0 + 1000 * MS,
        rosout,
        log_cdr(T0 + 1000 * MS, 20, "Publishing"),
    ));
    msgs.push((
        T0 + 2000 * MS,
        rosout,
        log_cdr(T0 + 2000 * MS, 40, "lost link to /dev/ttyUSB0"),
    ));
    msgs.sort_by_key(|m| m.0);
    for (seq, (t, channel_id, data)) in (1..).zip(msgs) {
        w.write_to_known_channel(
            &mcap::records::MessageHeader {
                channel_id,
                sequence: seq,
                log_time: t as u64,
                publish_time: t as u64,
            },
            &data,
        )
        .unwrap();
    }
    w.finish().unwrap();
}

/// A rosbag2 SQLite bag. Without `definitions` it is a Humble bag (no definitions stored).
fn write_db3(
    path: &Path,
    topic: &str,
    ty: &str,
    definition: Option<&str>,
    messages: &[(i64, Vec<u8>)],
) {
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(
        "CREATE TABLE topics(id INTEGER PRIMARY KEY, name TEXT NOT NULL, type TEXT NOT NULL,
            serialization_format TEXT NOT NULL, offered_qos_profiles TEXT NOT NULL);
         CREATE TABLE messages(id INTEGER PRIMARY KEY, topic_id INTEGER NOT NULL,
            timestamp INTEGER NOT NULL, data BLOB NOT NULL);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO topics VALUES (1, ?1, ?2, 'cdr',
            '- history: 3\n  depth: 0\n  reliability: 1\n  durability: 1')",
        rusqlite::params![topic, ty],
    )
    .unwrap();
    if let Some(def) = definition {
        db.execute_batch(
            "CREATE TABLE message_definitions(id INTEGER PRIMARY KEY, topic_type TEXT NOT NULL,
                encoding TEXT NOT NULL, encoded_message_definition TEXT NOT NULL,
                type_description_hash TEXT NOT NULL);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO message_definitions VALUES (1, ?1, 'ros2msg', ?2, '')",
            rusqlite::params![ty, def],
        )
        .unwrap();
    }
    for (t, data) in messages {
        db.execute(
            "INSERT INTO messages (topic_id, timestamp, data) VALUES (1, ?1, ?2)",
            rusqlite::params![t, data],
        )
        .unwrap();
    }
}

fn table_batch() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("t", DataType::Float64, false),
            Field::new("temp", DataType::Float64, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![
                1_700_000_000.0,
                1_700_000_000.5,
                1_700_000_001.0,
            ])),
            Arc::new(Float64Array::from(vec![20.0, 21.0, 25.5])),
        ],
    )
    .unwrap()
}

struct Fixture {
    _dir: tempfile::TempDir,
    bags: PathBuf,
    root: Root,
    store: Store,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let bags = dir.path().join("bags");
    std::fs::create_dir_all(bags.join("jazzy")).unwrap();
    write_mcap(&bags.join("run1.mcap"), 0.0);
    write_mcap(&bags.join("run2.mcap"), 1000.0);
    // Humble: sensor_msgs/Temperature from the bundled definitions.
    let temps: Vec<(i64, Vec<u8>)> = (0..50)
        .map(|i| {
            (
                T0 + i * 20 * MS,
                cdr(T0 + i * 20 * MS, "probe", &[30.0 + i as f64, 0.5]),
            )
        })
        .collect();
    write_db3(
        &bags.join("humble.db3"),
        "/temperature",
        "sensor_msgs/msg/Temperature",
        None,
        &temps,
    );
    // A newer rosbag2 folder storing the definition of a custom type.
    std::fs::write(
        bags.join("jazzy/metadata.yaml"),
        "rosbag2_bagfile_information:\n",
    )
    .unwrap();
    write_db3(
        &bags.join("jazzy/jazzy_0.db3"),
        "/sample",
        "my_msgs/msg/Sample",
        Some(SAMPLE_DEF),
        &sample_messages(5000.0),
    );
    std::fs::write(
        bags.join("bench.csv"),
        "time_s;voltage\n0.0;12.5\n0.1;12.4\n0.2;11.9\n",
    )
    .unwrap();
    // A logger that never updated its clock.
    std::fs::write(bags.join("frozen.csv"), "time_s;v\n5;1\n5;2\n5;3\n").unwrap();
    std::fs::write(
        bags.join("log.jsonl"),
        "{\"timestamp_ms\": 1000, \"v\": 1.5}\n{\"timestamp_ms\": 2000, \"v\": 2.5}\n",
    )
    .unwrap();
    let batch = table_batch();
    let mut pq = datafusion::parquet::arrow::ArrowWriter::try_new(
        std::fs::File::create(bags.join("table.parquet")).unwrap(),
        batch.schema(),
        None,
    )
    .unwrap();
    pq.write(&batch).unwrap();
    pq.close().unwrap();
    let mut ipc = datafusion::arrow::ipc::writer::FileWriter::try_new(
        std::fs::File::create(bags.join("table.arrow")).unwrap(),
        &batch.schema(),
    )
    .unwrap();
    ipc.write(&batch).unwrap();
    ipc.finish().unwrap();
    std::fs::write(bags.join("old.bag"), "#ROSBAG V2.0\n").unwrap();
    std::fs::write(bags.join("corrupt.mcap"), "this is not an MCAP file").unwrap();

    let root = Root {
        name: "bags".into(),
        path: dunce_like(&bags),
        include: vec![],
        exclude: vec![],
        origin: Origin::Default,
    };
    // What the engineer said: the root, a folder, a recording.
    let context = dir.path().join("context.toml");
    std::fs::write(
        &context,
        "[\"bags:\"]\nhardware = \"rover-b\"\n\n[\"bags:jazzy/\"]\nhardware = \"rover-c\"\n\n[\"bags:run2.mcap\"]\nwind = \"gusty\"\n",
    )
    .unwrap();
    let store = Store {
        context_file: Some(context),
        ..Store::new(&dir.path().join("state"))
    };
    Fixture {
        bags,
        root,
        store,
        _dir: dir,
    }
}

fn dunce_like(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

fn discover(f: &Fixture) -> Vec<Candidate> {
    let found = roots::candidates(&f.root, 10_000, Duration::from_secs(10)).candidates;
    f.store.write_candidates(&found).unwrap();
    found
}

async fn ingest_all(f: &Fixture) -> BTreeMap<String, Outcome> {
    let mut out = BTreeMap::new();
    for c in discover(f) {
        if c.kind.supported() {
            let o = store::ingest(&f.store, &f.root, &c).await.unwrap();
            out.insert(c.path.display().to_string(), o);
        }
    }
    out
}

/// The rows as objects keyed by column.
async fn q(f: &Fixture, sql_text: &str) -> Vec<Value> {
    let r = sql::query(&f.store, sql_text, None)
        .await
        .unwrap_or_else(|e| panic!("{sql_text}\n→ {e}"));
    r.rows
        .into_iter()
        .map(|row| Value::Object(r.columns.iter().cloned().zip(row).collect()))
        .collect()
}

async fn one(f: &Fixture, sql_text: &str) -> Value {
    let rows = q(f, sql_text).await;
    assert_eq!(rows.len(), 1, "{sql_text}: {rows:?}");
    rows[0].clone()
}

fn hashes(dir: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(hashes(&p));
        } else {
            out.insert(p.clone(), wb_core::fsutil::file_sha256(&p).unwrap());
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn every_format_is_extracted_and_queryable() {
    let f = fixture();
    let before = hashes(&f.bags);
    let outcomes = ingest_all(&f).await;
    assert_eq!(hashes(&f.bags), before, "recordings must never be modified");
    for (path, o) in &outcomes {
        if path == "corrupt.mcap" {
            assert!(matches!(o, Outcome::Failed(_)), "{path}: {o:?}");
        } else {
            assert!(matches!(o, Outcome::Extracted { .. }), "{path}: {o:?}");
        }
    }
    // A second pass finds everything up to date.
    let again = ingest_all(&f).await;
    assert!(again.values().all(|o| *o == Outcome::UpToDate), "{again:?}");

    let statuses = q(&f, "SELECT path, status FROM recordings ORDER BY path").await;
    let status = |p: &str| {
        statuses
            .iter()
            .find(|r| r["path"] == p)
            .map(|r| r["status"].clone())
            .unwrap_or(Value::Null)
    };
    assert_eq!(status("run1.mcap"), "done");
    assert_eq!(status("jazzy"), "done");
    assert_eq!(status("old.bag"), "unsupported");
    assert_eq!(status("corrupt.mcap"), "failed");

    // What the engineer said: for each key, the most specific target wins.
    let rows = q(
        &f,
        "SELECT recording, context FROM recordings WHERE recording IN ('bags:jazzy', 'bags:run1.mcap', 'bags:run2.mcap') ORDER BY 1",
    )
    .await;
    assert_eq!(
        rows,
        vec![
            json!({"recording": "bags:jazzy", "context": {"hardware": "rover-c"}}),
            json!({"recording": "bags:run1.mcap", "context": {"hardware": "rover-b"}}),
            json!({"recording": "bags:run2.mcap", "context": {"hardware": "rover-b", "wind": "gusty"}}),
        ]
    );

    // Values come back exactly.
    let r = one(
        &f,
        "SELECT count(*) AS n, min(x) AS lo, max(x) AS hi FROM stream('/sample', 'bags:run1%')",
    )
    .await;
    assert_eq!(r, json!({"n": 81, "lo": 0.0, "hi": 99.0}));
    // The planted gap, with lead().
    let r = one(
        &f,
        "SELECT max(next_ns - t_ns) AS gap_ns FROM (
           SELECT CAST(t AS BIGINT) AS t_ns, lead(CAST(t AS BIGINT)) OVER (ORDER BY t) AS next_ns
           FROM stream('/sample', 'bags:run1%'))",
    )
    .await;
    assert_eq!(r["gap_ns"], 2_000_000_000i64);
    // Rate per window, with date_bin: 8 full one-second windows of 10 messages.
    let r = one(
        &f,
        "SELECT count(*) AS full_windows FROM (
           SELECT date_bin(INTERVAL '1 second', t) AS w, count(*) AS n
           FROM stream('/sample', 'bags:run1%') GROUP BY w) WHERE n = 10",
    )
    .await;
    assert_eq!(r["full_windows"], 8);
    // header.stamp becomes t_sensor (5 ms before the receive time).
    let r = one(
        &f,
        "SELECT count(*) AS n FROM stream('/sample', 'bags:run1%')
         WHERE CAST(t AS BIGINT) - CAST(t_sensor AS BIGINT) = 5000000",
    )
    .await;
    assert_eq!(r["n"], 81);
    // A join between streams on time buckets.
    let r = one(
        &f,
        "SELECT count(*) AS shared FROM
           (SELECT date_bin(INTERVAL '1 second', t) AS w FROM stream('/camera/image', 'bags:run1%') GROUP BY w) a
           JOIN (SELECT date_bin(INTERVAL '1 second', t) AS w FROM stream('/sample', 'bags:run1%') GROUP BY w) b
           ON a.w = b.w",
    )
    .await;
    assert_eq!(r["shared"], 9);
    // The same metric across every recording of a hardware.
    let rows = q(
        &f,
        "SELECT s.recording, max(x) AS hi FROM stream('/sample') s
         JOIN recordings r ON s.recording = r.recording
         WHERE r.context['hardware'] = 'rover-b' GROUP BY s.recording ORDER BY s.recording",
    )
    .await;
    assert_eq!(
        rows,
        vec![
            json!({"recording": "bags:run1.mcap", "hi": 99.0}),
            json!({"recording": "bags:run2.mcap", "hi": 1099.0}),
        ]
    );
    // Images keep their timing and size only.
    let r = one(
        &f,
        "SELECT decoded, columns, notes FROM streams WHERE recording = 'bags:run1.mcap' AND stream = '/camera/image'",
    )
    .await;
    assert_eq!(
        (&r["decoded"], &r["columns"]),
        (&json!(false), &json!("t, message_bytes"))
    );
    let r = one(
        &f,
        "SELECT min(message_bytes) AS b, count(*) AS n FROM stream('/camera/image', 'bags:run1%')",
    )
    .await;
    assert_eq!(r, json!({"b": 1000, "n": 20}));
    // Each column comes with its type, even without rows.
    let r = sql::query(
        &f.store,
        "SELECT recording, t, message_bytes, message_bytes > 0 AS sent FROM stream('/camera/image') LIMIT 0",
        None,
    )
    .await
    .unwrap();
    assert_eq!(r.types, ["Utf8", "Timestamp(ns)", "UInt32", "Boolean"]);
    // Humble bag: decoded with the bundled sensor_msgs/Temperature.
    let r = one(
        &f,
        "SELECT count(*) AS n, max(temperature) AS hi, min(variance) AS var FROM stream('/temperature')",
    )
    .await;
    assert_eq!(r, json!({"n": 50, "hi": 79.0, "var": 0.5}));
    // Table-like files: the time column becomes t.
    let r = one(
        &f,
        "SELECT CAST(max(t) AS BIGINT) AS last_ns, max(voltage) AS v FROM stream('table', 'bags:bench.csv')",
    )
    .await;
    assert_eq!(r, json!({"last_ns": 200_000_000, "v": 12.5}));
    let r = one(
        &f,
        "SELECT CAST(min(t) AS BIGINT) AS ns FROM stream('table', 'bags:log.jsonl')",
    )
    .await;
    assert_eq!(r["ns"], 1_000_000_000i64);
    for file in ["table.parquet", "table.arrow"] {
        let r = one(
            &f,
            &format!("SELECT CAST(max(t) AS BIGINT) AS ns, max(temp) AS hi FROM stream('table', 'bags:{file}')"),
        )
        .await;
        assert_eq!(
            r,
            json!({"ns": 1_700_000_001_000_000_000i64, "hi": 25.5}),
            "{file}"
        );
    }

    // Warnings and errors of a log topic are counted in the recording's notes.
    let r = one(
        &f,
        "SELECT notes FROM recordings WHERE recording = 'bags:run1.mcap'",
    )
    .await;
    assert_eq!(
        r["notes"],
        "1 ERROR log messages (stream /rosout), first: \"lost link to /dev/ttyUSB0\""
    );
    let r = one(
        &f,
        "SELECT count(*) AS n, max(level) AS worst FROM stream('/rosout', 'bags:run1%')",
    )
    .await;
    assert_eq!(r, json!({"n": 2, "worst": 40}));

    // The publishers' QoS, in short.
    let rows = q(
        &f,
        "SELECT recording, stream, qos FROM streams WHERE qos IS NOT NULL AND recording IN ('bags:run1.mcap', 'bags:humble.db3') ORDER BY 1, 2",
    )
    .await;
    assert_eq!(
        rows,
        vec![
            json!({"recording": "bags:humble.db3", "stream": "/temperature", "qos": "reliable, transient_local"}),
            json!({"recording": "bags:run1.mcap", "stream": "/sample", "qos": "best_effort, volatile, depth 10"}),
        ]
    );

    // Facts the recording states about itself.
    let rows = q(
        &f,
        "SELECT key, value FROM properties WHERE recording = 'bags:run1.mcap'",
    )
    .await;
    assert_eq!(rows, vec![json!({"key": "rig.serial", "value": "R-17"})]);

    // A time that never changes is flagged.
    let r = one(
        &f,
        "SELECT notes FROM streams WHERE recording = 'bags:frozen.csv'",
    )
    .await;
    assert!(
        r["notes"]
            .as_str()
            .unwrap()
            .contains("t is the same on every row"),
        "{r}"
    );

    // Every stream is a Parquet file with page statistics on `t`.
    let rows = f.store.rows();
    let run1 = rows
        .iter()
        .find(|r| r.recording == "bags:run1.mcap")
        .unwrap();
    let meta = run1.meta.as_ref().unwrap();
    let sample = meta.streams.iter().find(|s| s.name == "/sample").unwrap();
    let file = std::fs::File::open(run1.dir.as_ref().unwrap().join(&sample.dataset)).unwrap();
    let reader = parquet::file::serialized_reader::SerializedFileReader::new(file).unwrap();
    use parquet::file::reader::FileReader;
    let md = reader.metadata();
    let t = md
        .file_metadata()
        .schema_descr()
        .columns()
        .iter()
        .position(|c| c.name() == "t")
        .unwrap();
    assert!(
        md.row_group(0).column(t).statistics().is_some(),
        "no statistics on t"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sql_is_locked_down_and_capped() {
    let f = fixture();
    ingest_all(&f).await;
    let outside = f.bags.join("bench.csv").display().to_string();
    for bad in [
        "CREATE TABLE x (a INT)".to_string(),
        "INSERT INTO recordings (recording) VALUES ('x')".into(),
        "DROP TABLE recordings".into(),
        "SET datafusion.execution.batch_size = 1".into(),
        format!("CREATE EXTERNAL TABLE e STORED AS CSV LOCATION '{outside}'"),
        format!("COPY (SELECT 1) TO '{}'", f.bags.join("copy.csv").display()),
        format!("SELECT * FROM '{outside}'"),
        format!("SELECT * FROM read_csv('{outside}')"),
        "SELECT 1; SELECT 2".into(),
        "SELECT * FROM stream('/nope')".into(),
    ] {
        let e = sql::query(&f.store, &bad, None).await.expect_err(&bad);
        assert_eq!(e.kind(), ErrorKind::QueryFailed, "{bad}: {e}");
    }
    assert!(!f.bags.join("copy.csv").exists());
    // Results are capped by size, and by `limit` when the agent sets one.
    let big = "SELECT value, repeat('x', 100) AS s FROM generate_series(1, 100000)";
    let r = sql::query(&f.store, big, None).await.unwrap();
    assert!(r.truncated && r.row_count > 100, "{}", r.row_count);
    assert!(serde_json::to_vec(&r.rows).unwrap().len() <= sql::MAX_BYTES);
    assert!(r.hint.unwrap().contains("KB"));
    let r = sql::query(&f.store, big, Some(3)).await.unwrap();
    assert_eq!((r.row_count, r.truncated), (3, true));
    assert!(r.hint.unwrap().contains("limit 3"));
    let r = sql::query(&f.store, "SELECT 1 AS a", Some(1))
        .await
        .unwrap();
    assert_eq!(
        (r.rows, r.truncated, r.hint),
        (vec![vec![json!(1)]], false, None)
    );
    // The hint gives the next page's condition when the result has `t`.
    let r = sql::query(
        &f.store,
        "SELECT t FROM stream('/sample', 'bags:run1%') ORDER BY t",
        Some(2),
    )
    .await
    .unwrap();
    assert!(
        r.hint.as_deref().unwrap().contains(&format!(
            "t > TIMESTAMP '{}'",
            r.rows[1][0].as_str().unwrap()
        )),
        "{:?}",
        r.hint
    );
    // Rows sharing one `t`: paging with `t >` would skip the rest of them.
    let r = sql::query(
        &f.store,
        "SELECT t, v FROM stream('table', 'bags:frozen.csv') ORDER BY t",
        Some(2),
    )
    .await
    .unwrap();
    let hint = r.hint.unwrap();
    assert!(
        hint.contains("share the same t") && !hint.contains("t > TIMESTAMP"),
        "{hint}"
    );
    // A broken context file is reported with its own code and fix.
    let context = f.store.context_file.clone().unwrap();
    let good = std::fs::read(&context).unwrap();
    std::fs::write(&context, "[oops\n").unwrap();
    let e = sql::query(&f.store, "SELECT 1", None).await.unwrap_err();
    assert_eq!(e.kind(), ErrorKind::ContextInvalid, "{e}");
    std::fs::write(&context, good).unwrap();
    assert_eq!(
        q(
            &f,
            "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY 1",
        )
        .await,
        vec![
            json!({"table_name": "properties"}),
            json!({"table_name": "recordings"}),
            json!({"table_name": "streams"})
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_and_deleted_recordings() {
    let f = fixture();
    ingest_all(&f).await;
    // A changed file gets a new fingerprint: pending, then extracted again.
    write_mcap(&f.bags.join("run2.mcap"), 2000.0);
    let entries = discover(&f);
    let run2: Candidate = entries
        .iter()
        .find(|c| c.path == Path::new("run2.mcap"))
        .unwrap()
        .clone();
    let pending: Vec<_> = f
        .store
        .pending(Duration::ZERO)
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(pending, vec![PathBuf::from("run2.mcap")]);
    assert!(matches!(
        store::ingest(&f.store, &f.root, &run2).await.unwrap(),
        Outcome::Extracted { .. }
    ));
    let r = one(
        &f,
        "SELECT max(x) AS hi FROM stream('/sample', 'bags:run2%')",
    )
    .await;
    assert_eq!(r["hi"], 2099.0);
    // A deleted file shows `missing`, and its data stays queryable.
    std::fs::remove_file(f.bags.join("run1.mcap")).unwrap();
    discover(&f);
    let r = one(
        &f,
        "SELECT status FROM recordings WHERE recording = 'bags:run1.mcap'",
    )
    .await;
    assert_eq!(r["status"], "missing");
    let r = one(
        &f,
        "SELECT count(*) AS n FROM stream('/sample', 'bags:run1%')",
    )
    .await;
    assert_eq!(r["n"], 81);
    let counts = f.store.counts();
    assert_eq!(counts.get(&Status::Missing), Some(&1));
}

#[tokio::test(flavor = "multi_thread")]
async fn documented_recipes_run() {
    let f = fixture();
    ingest_all(&f).await;
    for line in wb_data::tables::RECIPES.lines() {
        let Some((_, sql_text)) = line.split_once(": ") else {
            continue;
        };
        if !sql_text.starts_with("SELECT") && !sql_text.starts_with("WITH") {
            continue;
        }
        // Map the recipes' example names onto the fixture.
        let sql_text = sql_text
            .replace("'/scan'", "'/sample'")
            .replace("'/imu'", "'/sample'")
            .replace("\"linear_acceleration.z\"", "\"header.stamp.sec\"")
            .replace("'bags:run3'", "'bags:run1.mcap'")
            .replace("'%imu%'", "'%sample%'");
        let r = sql::query(&f.store, &sql_text, None)
            .await
            .unwrap_or_else(|e| panic!("recipe failed: {sql_text}\n→ {e}"));
        assert!(r.row_count > 0, "{sql_text}");
    }
}

/// The Peers and Similar recipes, as documented, on six CSV recordings of the same
/// signal: `r5` is offset by 5, `r1` is a near-copy of `r0`, and `r4` holds integers
/// (another type in the same column, as between firmware versions).
#[tokio::test(flavor = "multi_thread")]
async fn peer_recipes_rank_the_outlier_and_the_twin() {
    let dir = tempfile::tempdir().unwrap();
    let runs = dir.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    // Deterministic noise in [0, 0.1).
    let noise = |i: u64, seed: u64| ((i * 7919 + seed * 104_729) % 1000) as f64 / 10_000.0;
    for (name, seed, offset) in [
        ("r0", 0, 0.0),
        ("r1", 0, 0.001),
        ("r2", 1, 0.0),
        ("r3", 2, 0.0),
        ("r4", 3, 0.0),
        ("r5", 4, 5.0),
    ] {
        let mut csv = String::from("time_s;signal\n");
        for i in 0..200u64 {
            let v = (i as f64 * 0.1 + seed as f64).sin() + noise(i, seed) + offset;
            let v = if name == "r4" {
                format!("{}", v.round() as i64)
            } else {
                format!("{v}")
            };
            csv.push_str(&format!("{};{v}\n", i as f64 * 0.1));
        }
        std::fs::write(runs.join(format!("{name}.csv")), csv).unwrap();
    }
    let f = Fixture {
        root: Root {
            name: "peers".into(),
            path: dunce_like(&runs),
            include: vec![],
            exclude: vec![],
            origin: Origin::Default,
        },
        store: Store::new(&dir.path().join("state")),
        bags: runs,
        _dir: dir,
    };
    ingest_all(&f).await;
    let recipe = |label: &str| {
        wb_data::tables::RECIPES
            .lines()
            .find(|l| l.starts_with(&format!("- {label}")))
            .and_then(|l| l.split_once("): "))
            .unwrap()
            .1
            .replace("'/imu'", "'table'")
            .replace("\"linear_acceleration.z\"", "signal")
            .replace("'bags:run3'", "'peers:r0.csv'")
    };
    let peers = q(&f, &recipe("Peers")).await;
    assert_eq!(peers[0]["recording"], "peers:r5.csv", "{peers:?}");
    assert!(peers[0]["score"].as_f64().unwrap() > 3.0, "{peers:?}");
    let similar = q(&f, &recipe("Similar")).await;
    assert_eq!(similar[0]["recording"], "peers:r1.csv", "{similar:?}");
}
