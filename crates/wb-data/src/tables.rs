//! The tables the agent can query, documented once: for the `sql` tool description,
//! `docs/reference/tables.md`, and a test against the real schemas.

/// One documented column.
pub struct ColumnDoc {
    pub name: &'static str,
    pub ty: &'static str,
    pub description: &'static str,
}

/// One documented table (or table function).
pub struct TableDoc {
    pub name: &'static str,
    pub description: &'static str,
    pub columns: &'static [ColumnDoc],
}

macro_rules! cols {
    ($( $name:literal $ty:literal $desc:literal ),* $(,)?) => {
        &[ $( ColumnDoc { name: $name, ty: $ty, description: $desc }, )* ]
    };
}

pub const TABLES: &[TableDoc] = &[
    TableDoc {
        name: "recordings",
        description: "Every recording found in the data roots, with its extraction status.",
        columns: cols![
            "recording" "text" "Identifier `root:path`, used everywhere else",
            "root" "text" "Data root name",
            "path" "text" "Path inside the root (a folder for rosbag2)",
            "kind" "text" "`rosbag2`, `mcap`, `db3`, `ulog`, `csv`, `json_lines`, `parquet`, `arrow_ipc`, `tlog`, `ros1_bag`",
            "hardware" "text" "Hardware declared for the root (`roots add --hardware`)",
            "status" "text" "`done`, `pending`, `extracting`, `failed`, `paused` (low disk space), `missing` (file gone, data kept), `unsupported`",
            "error" "text" "Why extraction failed",
            "size_bytes" "bigint" "Size of the recording",
            "modified" "timestamp" "Last modification of the file",
            "start" "timestamp" "First `t` over all streams",
            "end" "timestamp" "Last `t` over all streams",
            "streams" "int" "Number of streams extracted",
            "notes" "text" "Remarks about the extraction",
        ],
    },
    TableDoc {
        name: "streams",
        description: "One row per stream of each extracted recording: topics (ROS 2), messages (ULog, MAVLink), or `table` (CSV, JSON Lines, Parquet, Arrow IPC).",
        columns: cols![
            "recording" "text" "`root:path`",
            "stream" "text" "Stream name, as given to `stream()`",
            "type" "text" "Message type (`sensor_msgs/msg/Imu`) or file format",
            "encoding" "text" "`cdr`, `ulog`, `table`…",
            "rows" "bigint" "Number of rows (messages)",
            "start" "timestamp" "First `t`",
            "end" "timestamp" "Last `t`",
            "decoded" "boolean" "`false` when only timing and size were kept (images, point clouds, unknown types)",
            "columns" "text" "Comma-separated column names",
            "notes" "text" "Remarks: what was left out and why",
        ],
    },
    TableDoc {
        name: "stream('<name>' [, '<recording LIKE pattern>'])",
        description: "Table function: the rows of one stream across all recordings (or those whose `recording` matches the LIKE pattern). Columns are the union over recordings; missing ones are null.",
        columns: cols![
            "recording" "text" "`root:path` of the row's recording",
            "t" "timestamp" "Receive time (MCAP log time, bag timestamp), ULog `timestamp` (time since boot, shown near 1970), the tlog record time, or the table's time column. Nanoseconds",
            "t_sensor" "timestamp" "ROS `header.stamp`, when the message has one",
            "message_bytes" "int" "Size of each message on the wire (bags and MCAP)",
            "<field>" "any" "Message fields, nested names flattened with dots: quote them, `\"linear_acceleration.z\"`. Arrays stay arrays (`unnest`). Table files keep their own columns",
        ],
    },
];

/// SQL recipes shown to the agent.
pub const RECIPES: &str = r#"- Gaps: SELECT t, (lead(t) OVER (ORDER BY t) - t) AS gap FROM stream('/scan') WHERE recording = 'bags:run3' ORDER BY gap DESC LIMIT 10
- Windows: SELECT date_bin(INTERVAL '100 milliseconds', t) AS w, count(*) AS n FROM stream('/scan') WHERE recording = 'bags:run3' GROUP BY w ORDER BY w
- Percentiles: SELECT approx_percentile_cont(0.99) WITHIN GROUP (ORDER BY "linear_acceleration.z") FROM stream('/imu')
- Join streams on time buckets: compute date_bin(...) in each stream, then JOIN ON the bucket and recording
- Names: SELECT * FROM streams WHERE stream ILIKE '%imu%' OR columns ILIKE '%current%'
- Plot (about 150 bins, width = (end - start) / 150 from streams): SELECT date_bin(INTERVAL '1 second', t) AS t, min("linear_acceleration.z") AS lo, avg("linear_acceleration.z") AS mean, max("linear_acceleration.z") AS hi FROM stream('/imu') WHERE recording = 'bags:run3' GROUP BY 1 ORDER BY 1
- Page by time window: SELECT t, "linear_acceleration.z" FROM stream('/imu') WHERE recording = 'bags:run3' AND t >= TIMESTAMP '1970-01-01T00:00:00' AND t < TIMESTAMP '2100-01-01T00:00:00' ORDER BY t
- Page by key, after the last row seen: SELECT recording, t, "linear_acceleration.z" FROM stream('/imu') WHERE recording > 'bags:run3' OR (recording = 'bags:run3' AND t > TIMESTAMP '1970-01-01T00:00:00') ORDER BY recording, t"#;

/// The tables, as Markdown (`docs/reference/tables.md`).
pub fn render_reference() -> String {
    let mut out = String::from(
        "<!-- GENERATED by `wisebucket docs gen`. Do not edit by hand. -->\n\n# Tables reference\n\n\
         What the agent's `sql` tool (and `wisebucket sql`) can query. Queries are read-only `SELECT`s \
         run by DataFusion over the store; see [the guide](../guide.md#the-store-and-sql).\n",
    );
    for t in TABLES {
        out.push_str(&format!(
            "\n## `{}`\n\n{}\n\n| Column | Type | Description |\n| --- | --- | --- |\n",
            t.name, t.description
        ));
        for c in t.columns {
            out.push_str(&format!(
                "| `{}` | {} | {} |\n",
                c.name, c.ty, c.description
            ));
        }
    }
    out.push_str("\n## Recipes\n\n");
    for line in RECIPES.lines() {
        let (label, sql) = line
            .trim_start_matches("- ")
            .split_once(": ")
            .unwrap_or(("", line));
        out.push_str(&format!("- **{label}:** `{sql}`\n"));
    }
    out
}

/// The tables, compactly, for the tool description.
pub fn summary() -> String {
    let mut out = String::new();
    for t in TABLES {
        out.push_str(&format!(
            "{}: {} Columns: {}.\n",
            t.name,
            t.description,
            t.columns
                .iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out
}
