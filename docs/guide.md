# User guide

Everything you need to install Wise Bucket, point it at your recordings, connect it to your AI agent, and query what it extracts.

- [Install](#install)
- [Set up](#set-up)
- [Recording folders](#recording-folders)
- [Connect your agent](#connect-your-agent)
- [The store and SQL](#the-store-and-sql)
- [Viewers](#viewers)
- [Everyday use](#everyday-use)
- [Where things live](#where-things-live)
- [Reset](#reset)
- [Uninstall](#uninstall)

When something goes wrong, see [Troubleshooting and FAQ](troubleshooting.md). Every command, setting and table is listed in the [CLI](reference/cli.md), [configuration](reference/configuration.md) and [tables](reference/tables.md) references.

## Install

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/MBagory/wise-bucket/releases/latest/download/wisebucket-installer.sh | sh
```

The installer puts `wisebucket` and its short alias `wbk` in `~/.cargo/bin` and adds that directory to your `PATH`. Then restart your shell, or run `source ~/.cargo/env` (`source ~/.cargo/env.fish` in fish), and `wisebucket --version` prints the version. To update, run the installer again. You also need an MCP-capable agent: Claude Code, OpenAI Codex, Cursor, Kilo Code, Cline…

| | Requirement |
| --- | --- |
| macOS | Apple Silicon or Intel |
| Linux | x86_64 or arm64, any distribution |
| Windows | through [WSL2](#windows-wsl2) |
| Disk | room for the store: a decoded copy of your recordings' signals (see [The store and SQL](#the-store-and-sql)) |

### Build from source

You need [Rust](https://rustup.rs) 1.96 or newer and a C compiler:

```sh
xcode-select --install                                              # macOS
sudo apt install build-essential                                    # Debian, Ubuntu
sudo dnf install gcc                                                # RHEL family, Fedora

cargo install --git https://github.com/MBagory/wise-bucket wisebucket --locked
```

The build takes several minutes. It puts `wisebucket` and `wbk` in `~/.cargo/bin` too, which rustup already added to your `PATH`. To update, run it again.

### Windows: WSL2

Native Windows support is a possible feature. Today, run Wise Bucket inside WSL2, where it behaves exactly as on Linux:

1. Install WSL2 with Ubuntu: `wsl --install -d Ubuntu`.
2. Inside Ubuntu, run the install commands above.
3. Run your agent **from WSL** (for example, start `claude` in the WSL terminal, or use VS Code's WSL remote) so it can launch the Linux binary.
4. Recordings on the Windows side are reachable as `/mnt/c/...` and can be declared as recording folders. Access through `/mnt/c` is slower; for large recordings, prefer a folder inside the WSL file system.

## Set up

Run this once per machine:

```text
$ wisebucket setup
Wise Bucket setup
✔ state directory: ~/.local/share/wisebucket
? Add a folder containing recordings (ROS 2 bags, MCAP, ULog, CSV…)? › yes
  Folder path › ~/robot-logs/bags
  Short name › bags
  Hardware these recordings come from (robot, vehicle…; optional) › rover-b
✔ root "bags" → /home/me/robot-logs/bags · 37 recording(s) (6 rosbag2 folders, 31 MCAP)
? Add another folder? › no
✔ configuration written to ~/.config/wisebucket/config.toml

Connect your agent (once, for all your folders). Claude Code:
claude mcp add --scope user wise-bucket -- /home/me/.cargo/bin/wisebucket serve
Other agents (Kilo Code, Cline, Cursor, …): add this to their global MCP settings:
{ "mcpServers": { "wise-bucket": { "command": "/home/me/.cargo/bin/wisebucket", "args": ["serve"] } } }
```

`setup` creates your user configuration and the state directory, and records your answers. There is nothing to download and no database to install. Re-run it any time: it only does what's missing.

**Non-interactive** (scripts, CI):

```sh
wisebucket setup --yes --root bags=~/robot-logs/bags --root flights=/mnt/nas/px4
```

## Recording folders

A recording folder (a *data root*) is a named folder that Wise Bucket may read. It can hold recordings at any depth:

| Recording | Recognized as |
| --- | --- |
| A folder with `metadata.yaml` and `.mcap` or `.db3` splits | one ROS 2 bag (rosbag2) |
| `.mcap` | MCAP (ROS 2 messages are decoded; other encodings keep their timing and sizes) |
| `.db3` | a rosbag2 SQLite file |
| `.ulg` | PX4 ULog |
| `.tlog` | a MAVLink telemetry log (QGroundControl, MAVProxy): one stream per message |
| `.asc` and `.blf` | a Vector CAN log (CANoe, CANalyzer): every frame in one stream (`.asc` only when its first bytes match) |
| `.csv`, `.jsonl` / `.ndjson`, `.parquet`, `.arrow` / `.feather` / `.ipc` | a table |
| `.bag` | a ROS 1 bag: listed, not readable yet |
| `.bin` (ArduPilot DataFlash), `.mf4` (MDF4) | listed, not readable yet (`.bin` only when its first bytes match) |

```sh
wisebucket roots add bags ~/robot-logs/bags
wisebucket roots add flights /Volumes/lab-nas/px4   # network mounts work
wisebucket roots list
wisebucket roots check      # readable? overlapping? how many recordings?
wisebucket roots remove flights                                        # the folder itself is never touched
```

**Rules:**
- **Names** are short and stable (`bags`, `lab-nas`): lowercase letters, digits, `-` and `_`, at most 32 characters. Recordings are referenced as `name:relative/path` (`bags:2026-09-24/dock_run3`), so if a mount point moves, you only update that folder's path.
- **Include and exclude** (optional) are glob patterns on file paths: `--exclude '**/tmp/**'`. A rosbag2 folder is matched through its `metadata.yaml`.
- **No overlap:** a folder inside another declared folder is refused.
- **No escapes:** symlinks must stay inside their folder.
- **Read-only:** Wise Bucket never modifies, moves or deletes recordings.
- **Your decision only:** Wise Bucket gives your agent no tool to add a folder. You add folders from the command line or a configuration file.
- **Live:** folder changes reach a running agent session on its next call, no restart needed.

**No recordings yet?** `wisebucket demo` downloads public sample files (ROS 2 bags, MCAP, ULog, MAVLink, Parquet, and formats listed as `unsupported`) and declares them as the `demo` folder. See [Try it with sample data](../README.md#try-it-with-sample-data).

## Connect your agent

Run this once, after `setup`: Wise Bucket is registered for your user and works in every folder. There is nothing to do per repository; your recordings are organized by recording folder, not by code repository.

`setup` ends by printing the exact entry for this machine (binary path, plus `--config`/`--state-dir` if you passed them):

| Agent | How to connect |
| --- | --- |
| **Claude Code** | Run the printed `claude mcp add --scope user wise-bucket -- … serve` |
| **Kilo Code, Cline, Cursor** | Add the printed JSON entry to the agent's global MCP settings |
| **OpenAI Codex** | Add the entry to `~/.codex/config.toml` (below) |

```toml
# ~/.codex/config.toml
[mcp_servers.wise-bucket]
command = "/home/me/.cargo/bin/wisebucket"
args = ["serve"]
```

`setup` only prints these entries; it never edits your agent's settings. Re-run `setup` to see them again.

**Check the connection.** Start a new agent session in any folder (in Claude Code, `/mcp` lists `wise-bucket`). Then ask *"Is Wise Bucket ready?"*. The agent calls `server_info` and reports:
- the version
- the store: how many recordings are extracted, waiting or failed, and its size
- the recording folders it may use, with how many recordings each holds

Your agent gets two tools: `server_info`, and `sql` to query the store. Both are read-only. Optional, for Claude Code: to be asked before each Wise Bucket tool call, add this to `.claude/settings.json`:

```json
{ "permissions": { "ask": ["mcp__wise-bucket"] } }
```

## The store and SQL

**Wise Bucket trades disk space for query speed.** It reads each recording once and keeps a decoded, columnar copy of its signals: the *store*. Every question then becomes a SQL query that reads only the columns and time range it needs, in milliseconds to seconds, instead of a new pass over your bags. Why this design, and the alternatives that were set aside: [ADR 0012](decisions/0012-trade-storage-for-query-speed.md).

**What takes space, and what doesn't:**
- Numeric signals are stored compressed and usually take a fraction of the original file.
- Images, point clouds, maps and video are **not** copied: only their timing and message sizes are, so your agent can still check their rates and bandwidth.
- `server_info` and `doctor` show the store's size.
- Extraction **pauses** while the store's disk has less than 10 GB free, and resumes on its own when space comes back.
- The store is **derived data**: deleting it is safe (see [Reset](#reset)); it is extracted again.

### When recordings are extracted

While an agent session is open, Wise Bucket watches your folders for file events, and walks them at the start and each time the agent calls `server_info` ([ADR 0018](decisions/0018-extraction-on-file-events.md)). New and changed recordings are extracted in the background, newest first, one at a time, at low priority:

- A file or rosbag2 folder **renamed into place** (a finished download, `mv`) is extracted at once.
- A file **written in place** (`cp`, a logger) is extracted once it has not changed for 5 seconds.
- Changes made by another computer on a **network share** send no file event: they are found at the next `server_info` call. The same goes for a folder declared during the session.
- If a folder cannot be watched, the session walks your folders every minute instead.

Several agent windows share the work and never extract the same file twice.

Each recording has a **status**:

| Status | Meaning |
| --- | --- |
| `done` | Extracted: its streams can be queried |
| `pending` | Waiting to be extracted (only happens while a session is open) |
| `extracting` | Being extracted |
| `failed` | Could not be read; `error` says why. Retried when the file changes |
| `paused` | Waiting for free disk space |
| `missing` | The file is gone; its data stays queryable |
| `unsupported` | A format Wise Bucket cannot read yet (ROS 1 bags, ArduPilot DataFlash, MDF4) |

### What gets extracted

Every recording becomes one or more **streams**: the topics of a ROS 2 bag or MCAP file, the messages of a ULog file or a MAVLink telemetry log, or a single `table` for CSV, JSON Lines, Parquet and Arrow files. Each stream is a table with:

- **`t`**, a nanosecond timestamp: the receive time for bags and MCAP, `timestamp` for ULog (time since boot, so it shows as a date near 1970; in seconds: `date_part('epoch', t)`; a ULog recording starts at its header's time), the record time for a MAVLink telemetry log, the frame time for a CAN log, or the time column of a table.
- **`t_sensor`**, the message's own `header.stamp`, when a ROS 2 message has one.
- **`message_bytes`**, the size of each message (bags and MCAP).
- **The message fields**, nested names joined with dots (`linear_acceleration.z`). Arrays stay arrays.

**ROS 2 messages** are decoded from the definitions stored in the recording. ROS 2 Humble bags (`.db3`) don't store them, so standard types (`std_msgs`, `geometry_msgs`, `sensor_msgs`, `nav_msgs`, `diagnostic_msgs`, `trajectory_msgs`, `tf2_msgs`, `/rosout`) are decoded from definitions bundled with Wise Bucket. Other custom types in Humble bags keep their timing and sizes only. Each topic's QoS is in `streams.qos`, in short: `reliable, volatile, depth 10` (` | ` between publishers).

**ULog files** have one stream per message (`sensor_accel`). A second instance of a message is `sensor_accel.1`: pyulog's `sensor_accel` with multi_id 1, ulog2csv's `…_sensor_accel_1.csv`. Five more streams hold the log's own records, timed as pyulog does (records without a timestamp take the last data timestamp before them):

- **`parameters`**: `name`, `value`, `initial` (true for the values at the start of the log, false for changes during it), and `system_default` and `airframe_default`, recorded only when they differ from `value` (newer logs).
- **`logged_messages`**: `level` (`EMERGENCY`, `ALERT`, `CRITICAL`, `ERROR`, `WARNING`, `NOTICE`, `INFO`, `DEBUG`), `message`, and `tag` for tagged messages.
- **`dropouts`**: `duration_ms` of each logger dropout.
- **`texts`**: `name`, `line`, `text`: the boot console output, a crash dump from an earlier crash (`hardfault_plain`), performance counters, one row per line.
- **`formats`**: `name`, `fields` as the file declares them (`uint64_t timestamp;float pressure`), and `logged`, false for the message types the file defines but never logged.

**CAN logs** (`.asc`, `.blf`) have one stream, `frames`: `channel` as the log numbers it (CAN 1 is 1), `id` (empty for error frames), `extended`, `fd`, `remote`, `error`, `rx`, `dlc` and `data`, the payload as an array of bytes (`data[1]` is the first). In an ASC log, `t` counts from the start of the measurement, and the `date` it started, as written in the log, is in `properties`. In a BLF log, `t` is the start time in the file header, read as UTC, plus each frame's offset, to the microsecond. Lines that are not frames (statistics, J1939 transport) are counted in `recordings.notes`, and so are error frames. Remote frames written without a length (`2.510001 2 100 Tx r`) are not read.

**Facts about a recording** go to the `properties` table: the info of ULog files (`ver_hw` board, `ver_sw` firmware, `sys_name`, `time_ref_utc`…) and the metadata records of MCAP files, as `<record>.<key>`. The `rosbag2` summary record is left out.

**Notes** point to what to look at. `recordings.notes` counts the warnings and errors logged by ULog (`logged_messages`), ROS 2 `/rosout` (WARN or worse) and MAVLink `STATUSTEXT` (WARNING or worse), and quotes the first error, or the first warning when there is none: `4 ERROR log messages (stream logged_messages), first: "[sensors] no barometer found on /dev/baro0 (2)"`. It also reports ULog logger dropouts, crash dumps, CAN error frames, and a file cut or damaged; no note means none of these. `streams.notes` flags ULog timestamps that are not set (0) and a time that never changes in a message stream.

**Tables** need a time column. Wise Bucket uses the first timestamp column, else a column named `time`, `timestamp`, `t` or `stamp` (any case), whose unit comes from a `_s`, `_ms`, `_us` or `_ns` suffix (seconds by default). The stream's `notes` say which column was used. Without one, `t` is empty.

### What you know about recordings

Some facts are not in the files: the machine, what changed, the conditions, the purpose of a test. Tell your agent: it records them with its `note` tool, and they show in `recordings.context`.

```text
you   › The flights of October are drone-2, with the new 9x4.5 carbon propellers.

agent › ⏺ wise-bucket · note
          target: flights:2026-10/
          context: {"hardware": "drone-2", "propellers": "9x4.5 carbon"}
```

The same from the terminal:

```sh
wisebucket note flights: hardware=drone-2                       # every recording of a root
wisebucket note flights:2026-10/ propellers='9x4.5 carbon'      # every recording under a folder
wisebucket note flights:2026-10/flight3.ulg wind='gusts 8 m/s'  # one recording
wisebucket note flights:2026-10/flight3.ulg wind=               # remove a key
```

- **Targets:** a recording (`root:path`, as `recordings` lists it), a folder (`root:folder/`) or a root (`root:`). For each key, the most specific target wins.
- **`hardware`** is the key for the machine: your agent compares recordings of the same `context['hardware']`.
- **In SQL:** `SELECT recording, context FROM recordings WHERE context['hardware'] = 'drone-2'`.
- **Where:** `context.toml`, next to your settings file (`wisebucket config path`). You can edit it by hand: one table per target (`["flights:2026-10/"]`), then `key = "value"` lines. Your recordings are never modified.

### Querying

**Nothing is computed in advance**: there are no precomputed statistics or fixed windows. Your agent writes the SQL each question needs, at the question's time scale, and shows you the query behind each number; you can run the same queries with `wisebucket sql` ([ADR 0010](decisions/0010-metrics-are-sql-written-by-the-agent.md)). There are four things to query (full list of columns: [tables reference](reference/tables.md)):

- `recordings`: every recording, with its status, context, time range and notes (warnings and errors logged, dropouts, crash dumps)
- `streams`: every stream of every recording, with its row count, columns and notes
- `properties`: short facts each recording states about itself, as `key` and `value`
- `stream('<name>')`: the rows of one stream across all recordings, with a `recording` column; `stream('<name>', '<recording LIKE pattern>')` restricts the recordings

```sh
wisebucket sql "SELECT status, count(*) FROM recordings GROUP BY status"
wisebucket sql "SELECT recording, stream, rows FROM streams WHERE stream ILIKE '%imu%'"
wisebucket sql "SELECT recording, key, value FROM properties"
wisebucket sql "SELECT recording, name, value, system_default FROM stream('parameters') WHERE system_default IS NOT NULL"
wisebucket sql "SELECT date_bin(INTERVAL '1 second', t) AS w, count(*) FROM stream('/scan', 'bags:run3%') GROUP BY w ORDER BY w"
```

Queries are read-only `SELECT`s, run by [Apache DataFusion](https://datafusion.apache.org) (a dialect close to PostgreSQL). Each query is limited to 30 seconds and 1 GB of memory, and sees only the store. A result is capped at about 60 KB of rows, or `--limit N` rows; when rows are cut, a hint says how to get the rest. To go through a long stream, page by time window (`WHERE t > <last t> ORDER BY t`; when the last rows share one `t`, the hint pages by another column) or aggregate it with `date_bin` (see the [recipes](reference/tables.md#recipes)) ([ADR 0017](decisions/0017-result-size-and-paging.md)). Quote dotted and upper-case column names: `"linear_acceleration.z"`. Add `--json` for machine-readable output: `columns`, their `types` (`Float64`, `Boolean`, `Timestamp(ns)`…) and `rows`, as the agent's `sql` tool returns them.

### Abnormal and similar recordings

Your agent compares recordings with their **peers** (the same stream and column in other recordings) using three [recipes](reference/tables.md#recipes) ([ADR 0020](decisions/0020-abnormality-and-similarity-as-sql-recipes.md)):

- **Peers:** one value per recording (the 99th percentile, the rate, the longest gap, the standard deviation…), then a robust z-score across recordings: how many median absolute deviations (MAD) the value lies from the peers' median.
- **Spikes:** the same score inside one recording, against the last 200 samples.
- **Similar:** the 5th, 25th, 50th, 75th and 95th percentiles of a signal, compared with a reference recording's; the closest recordings come first.

```sh
wisebucket sql "WITH per AS (SELECT recording, approx_percentile_cont(0.99) WITHIN GROUP (ORDER BY \"accelerometer_m_s2[2]\") AS v FROM stream('sensor_combined') GROUP BY recording), m AS (SELECT *, median(v) OVER () AS med FROM per), d AS (SELECT *, median(abs(v - med)) OVER () AS mad FROM m) SELECT recording, v, (v - med) / (1.4826 * mad) AS score FROM d ORDER BY abs((v - med) / (1.4826 * mad)) DESC NULLS LAST"
```

Reading a score:

| Score | Meaning |
| --- | --- |
| Below 3 (absolute value) | Within the spread of the peers |
| 3 or more | Unusual against these peers |
| Empty or infinite | Fewer than 3 peers, or peers that all have the same value |

A score depends on the peers: compare recordings of the same hardware and the same kind of test (`context['hardware']` in `recordings`, a join with `properties` on the board `ver_hw` or the firmware `ver_sw`, or a `LIKE` on the recording). Scores on counters, sequence numbers, timestamps and enums are meaningless. The `ulog` demo set (`wisebucket demo ulog`) has six PX4 logs to try the recipes on.

### Opening the store yourself

The store is a set of [Parquet](https://parquet.apache.org) files, one per stream, readable from Python, MATLAB, DuckDB and most data tools. `recording.json` lists each stream's file:

```python
import json, pandas as pd
rec = json.load(open("<state directory>/store/<key>/<version>/recording.json"))
imu = next(s for s in rec["streams"] if s["name"] == "/imu")
df = pd.read_parquet(f"<state directory>/store/<key>/<version>/{imu['dataset']}")
```

```matlab
T = parquetread("<state directory>/store/<key>/<version>/data/0003.parquet");
```

`recording.json` also holds the recording's `properties`. Nested message fields are struct columns (`linear_acceleration` holds `x`, `y`, `z`). Prefer `wisebucket sql` for anything you can express in SQL: it finds the files for you and flattens the fields into dotted columns.

## Viewers

To see what your agent found, open the original recording in a viewer at the time it points to. These viewers open the same files Wise Bucket reads:

| Viewer | Opens | License | Agent control (MCP) | Get it |
| --- | --- | --- | --- | --- |
| [Rerun](https://rerun.io) | MCAP, including ROS 2 bags recorded as MCAP (experimental) | Open source (MIT, Apache-2.0) | [`rerun viewer-mcp`](https://rerun.io/docs/reference/viewer/mcp) | `pip3 install rerun-sdk` or `cargo binstall rerun-cli` ([all options](https://rerun.io/docs/getting-started/installing-viewer)) |
| [Foxglove](https://foxglove.dev) | MCAP, ROS 2 `.db3`, PX4 ULog | Proprietary, sign-in on first launch, free plan for small teams | [Desktop app MCP server](https://docs.foxglove.dev/docs/agents/mcp-server) (paid plan) | [Download](https://foxglove.dev/download) |
| [Lichtblick](https://github.com/lichtblick-suite/lichtblick) | MCAP, PX4 ULog | Open source (MPL-2.0) | None | [Web app](https://lichtblick-suite.github.io/lichtblick/) or [desktop releases](https://github.com/lichtblick-suite/lichtblick/releases) |
| [PlotJuggler](https://github.com/PlotJuggler/PlotJuggler) | MCAP, ROS 2 bags, PX4 ULog, CSV, Parquet | Open source (MPL-2.0) | None | [Releases](https://github.com/PlotJuggler/PlotJuggler/releases), `sudo snap install plotjuggler`, or `sudo apt install ros-$ROS_DISTRO-plotjuggler-ros` |

With a viewer's MCP server connected to the same agent, the agent can open the file and move to the right time itself.

### Sending query results to Rerun

Wise Bucket sends the rows of any query to a running Rerun viewer (start it with `rerun`). Your agent does it with the `rerun` tool; in the terminal:

```sh
wisebucket rerun --entity sensor_combined "SELECT t, \"accelerometer_m_s2[2]\" AS az FROM stream('sensor_combined')"
```

- The query must return a timestamp column `t`. Each timestamp column (`t`, `t_sensor`) becomes a Rerun timeline with the same name. Times before 2000 count from boot (PX4 ULog, MAVLink) and are shown as durations.
- Each numeric or boolean column becomes a scalar series at `<entity>/<column>`. Name series with SQL aliases (`AS "imu/az"`). Other columns are skipped.
- Every row is sent, without the 60 KB cap of `sql`, to the Rerun recording `wisebucket`. Sending the same query twice adds its rows twice.
- The viewer's address is `rerun+http://127.0.0.1:9876/proxy`; set another with `--url`. PlotJuggler's real site is its GitHub page: its README warns that `plotjuggler.com` is not theirs.

## Everyday use

| You want to… | Run |
| --- | --- |
| Check that everything works | `wisebucket doctor` |
| See what has been extracted | `wisebucket sql "SELECT status, count(*) FROM recordings GROUP BY status"` |
| See why a recording failed | `wisebucket sql "SELECT recording, error FROM recordings WHERE status = 'failed'"` |
| See your settings and where each one comes from | `wisebucket config show --origin` |
| Tell what the files don't say | `wisebucket note flights: hardware=drone-2` |

- **Several windows at once:** each agent window starts its own Wise Bucket process. They share the store and the extraction work.
- **Machine-readable output:** `config`, `roots`, `sql` and `doctor` accept `--json`.

## Where things live

| What | Where |
| --- | --- |
| Your settings and recording folders | Linux `~/.config/wisebucket/config.toml`, macOS `~/Library/Application Support/wisebucket/config.toml` |
| What you said about recordings | `context.toml`, next to your settings file |
| How your agent starts Wise Bucket | Your agent's user-level MCP settings (e.g. `claude mcp get wise-bucket`) |
| State directory | Linux `~/.local/share/wisebucket/`, macOS `~/Library/Application Support/wisebucket/`. To use another disk (the store grows with your recordings), set `state_dir = "…"` in your settings |

```text
<state directory>/
├── store/            the decoded copy of your recordings (derived; safe to delete)
│   ├── candidates.json        the recordings found in your folders
│   └── <key>/<version>/       one recording: recording.json, data/NNNN.parquet (one file per stream)
├── demo/             sample recordings from `wisebucket demo`
└── logs/             server.log
```

**Settings precedence**, highest first: command-line flags > `WB_*` environment variables > user configuration > defaults. Annotated example: [user configuration](examples/user-config.toml).

## Reset

To rebuild the store from scratch, close your agent sessions, then:

```sh
wisebucket config path        # shows the state directory
rm -rf "<state directory>/store"
```

The next agent session extracts everything again. Your recordings, settings and `context.toml` are untouched.

## Uninstall

```sh
claude mcp remove --scope user wise-bucket    # or remove the entry from your agent's MCP settings
rm ~/.cargo/bin/wisebucket ~/.cargo/bin/wbk   # built from source: cargo uninstall wisebucket
```

Your store stays in the state directory (`wisebucket config path` shows where). Delete it yourself if you no longer need it; Wise Bucket never deletes it. On macOS your settings and `context.toml` live in the same folder, so deleting it removes them too. Your recordings are never touched.
