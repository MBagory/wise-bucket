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
            "kind" "text" "`rosbag2`, `mcap`, `db3`, `ulog`, `csv`, `json_lines`, `parquet`, `arrow_ipc`, `tlog`, `can_asc`, `can_blf`, `ardupilot_bin`, `nmea0183`, `nmea2000`, `mdf4`; listed but not readable yet: `ros1_bag`",
            "context" "map" "What the engineer said about the recording (`note` tool), key → value: `context['hardware']` is the machine; `context['dbc']` names the DBC files of CAN logs (`[<channel>=]<root:path>` separated by `;`, or `none`). For each key, the most specific target wins: the recording, then its folders, then its root",
            "status" "text" "`done`, `pending`, `extracting`, `failed`, `paused` (low disk space), `missing` (file gone, data kept), `unsupported`",
            "error" "text" "Why extraction failed",
            "size_bytes" "bigint" "Size of the recording",
            "modified" "timestamp" "Last modification of the file",
            "start" "timestamp" "First `t` over all streams; for ULog, the log header's start time; for ArduPilot, the UTC time of boot from its first GPS fix",
            "end" "timestamp" "Last `t` over all streams",
            "streams" "int" "Number of streams extracted",
            "notes" "text" "What to look at first: warnings and errors logged (counted per level, with the first message), ULog logger dropouts and crash dumps, and remarks about the extraction (a file cut or damaged). No note means none of these",
        ],
    },
    TableDoc {
        name: "streams",
        description: "One row per stream of each extracted recording: topics (ROS 2; MCAP protobuf and JSON: one column per field, `bytes` fields as their length), messages (ULog, MAVLink, ArduPilot DataFlash: one stream per message type, `PARM`, `MSG`, `ERR` and `EV` included; units in `notes`), `frames` (CAN logs: `channel` as the log numbers it, `id` (null for error frames), `extended`, `fd`, `remote`, `error`, `rx`, `dlc`, `data` as an array of bytes), `sentences` (NMEA 0183: every line, `talker`, `type`, `text`, `valid` checksum) and one stream per sentence type it decodes (`GGA`, `RMC`…), NMEA 2000 `frames` (`pgn`, `src`, `dst`, `prio`, `data`) and one stream per PGN decoded, named as canboat names it (`windData`, type `PGN 130306`; fields in SI units, listed in `notes`; repeating sets as `list`), MDF4 channel groups (named by their acquisition name, else `group_<n>`: one column per channel, physical values; units in `notes`), `<dbc>/<message>` (CAN signals decoded with a confirmed DBC: `channel`, one column per signal, `<signal>_label` for value tables; units in `notes`), or `table` (CSV, JSON Lines, Parquet, Arrow IPC). ULog files also have `parameters` (`name`, `value`, `initial`: true for the values at the start of the log, false for changes during it; `system_default` and `airframe_default` are recorded only when they differ from `value`: `system_default IS NOT NULL` lists the parameters changed from their default), `logged_messages` (`level` EMERGENCY…DEBUG, `message`, `tag`), `dropouts` (`duration_ms`) and `texts` (`name`, `line`, `text`: boot console, crash dump, performance counters; filter with ILIKE, page by `line`) and `formats` (`name`, `fields` as the file declares them, `logged`: false for message types defined but never logged). ROS 2 `/rosout` `level`: 10 DEBUG, 20 INFO, 30 WARN, 40 ERROR, 50 FATAL.",
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
            "qos" "text" "ROS 2 bags: the publishers' QoS in short (`reliable, volatile, depth 10`; ` | ` between publishers)",
        ],
    },
    TableDoc {
        name: "properties",
        description: "Short facts each recording states about itself: ULog info (`ver_hw` board, `ver_sw` firmware, `sys_name`, `sys_os_name`, `time_ref_utc`…), MCAP metadata records (`<record>.<key>`), ArduPilot `VER.<field>` and `firmware` (the first `MSG` naming it), the `date` of a CAN ASC log, and the MDF4 header's `comment` and properties (`HD.<tree>/<name>`).",
        columns: cols![
            "recording" "text" "`root:path`",
            "key" "text" "Name of the fact",
            "value" "text" "Its value, as text",
        ],
    },
    TableDoc {
        name: "stream('<name>' [, '<recording LIKE pattern>'])",
        description: "Table function: the rows of one stream across all recordings (or those whose `recording` matches the LIKE pattern). Columns are the union over recordings; missing ones are null.",
        columns: cols![
            "recording" "text" "`root:path` of the row's recording",
            "t" "timestamp" "Receive time (MCAP log time, bag timestamp), ULog `timestamp` (time since boot, shown near 1970; seconds: `date_part('epoch', t)`), the tlog record time, ArduPilot `TimeUS` (time since boot), NMEA 0183 UTC times on the date of the latest `RMC` or `ZDA`, NMEA 2000 capture times, MDF4 start time plus the group's time master, CAN frame times (ASC: since the start of the measurement; BLF: the file's start time, read as UTC, plus each frame's offset), or the table's time column. Nanoseconds",
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
- Properties: SELECT key, value FROM properties WHERE recording = 'bags:run3'
- Context (what the engineer said): SELECT recording, context FROM recordings WHERE context['hardware'] = 'rover-b'
- Plot (about 150 bins, width = (end - start) / 150 from streams): SELECT date_bin(INTERVAL '1 second', t) AS t, min("linear_acceleration.z") AS lo, avg("linear_acceleration.z") AS mean, max("linear_acceleration.z") AS hi FROM stream('/imu') WHERE recording = 'bags:run3' GROUP BY 1 ORDER BY 1
- Page by time window: SELECT t, "linear_acceleration.z" FROM stream('/imu') WHERE recording = 'bags:run3' AND t >= TIMESTAMP '1970-01-01T00:00:00' AND t < TIMESTAMP '2100-01-01T00:00:00' ORDER BY t
- Page by key, after the last row seen: SELECT recording, t, "linear_acceleration.z" FROM stream('/imu') WHERE recording > 'bags:run3' OR (recording = 'bags:run3' AND t > TIMESTAMP '1970-01-01T00:00:00') ORDER BY recording, t
- Peers (robust z-score of one value per recording; 3 or more is unusual against these peers): WITH per AS (SELECT recording, approx_percentile_cont(0.99) WITHIN GROUP (ORDER BY "linear_acceleration.z") AS v FROM stream('/imu') GROUP BY recording), m AS (SELECT *, median(v) OVER () AS med FROM per), d AS (SELECT *, median(abs(v - med)) OVER () AS mad FROM m) SELECT recording, v, (v - med) / (1.4826 * mad) AS score FROM d ORDER BY abs((v - med) / (1.4826 * mad)) DESC NULLS LAST LIMIT 10
- Spikes (the same score over the last 200 samples): WITH a AS (SELECT t, "linear_acceleration.z" AS x, median("linear_acceleration.z") OVER (ORDER BY t ROWS BETWEEN 200 PRECEDING AND CURRENT ROW) AS med FROM stream('/imu') WHERE recording = 'bags:run3'), b AS (SELECT *, median(abs(x - med)) OVER (ORDER BY t ROWS BETWEEN 200 PRECEDING AND CURRENT ROW) AS mad FROM a) SELECT t, x, (x - med) / (1.4826 * mad) AS score FROM b WHERE mad > 0 ORDER BY abs((x - med) / (1.4826 * mad)) DESC LIMIT 10
- Similar (closest distribution to a reference recording): WITH q AS (SELECT recording, make_array(approx_percentile_cont(0.05) WITHIN GROUP (ORDER BY "linear_acceleration.z"), approx_percentile_cont(0.25) WITHIN GROUP (ORDER BY "linear_acceleration.z"), approx_percentile_cont(0.5) WITHIN GROUP (ORDER BY "linear_acceleration.z"), approx_percentile_cont(0.75) WITHIN GROUP (ORDER BY "linear_acceleration.z"), approx_percentile_cont(0.95) WITHIN GROUP (ORDER BY "linear_acceleration.z")) AS v FROM stream('/imu') GROUP BY recording) SELECT o.recording, array_distance(o.v, r.v) AS distance FROM q o JOIN q r ON r.recording = 'bags:run3' WHERE o.recording <> r.recording ORDER BY distance LIMIT 10"#;

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
