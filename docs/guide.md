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

There is no release yet: prebuilt binaries come with the first release ([ADR 0015](decisions/0015-first-release-after-m1.md)). Until then, build from source. You need [Rust](https://rustup.rs) 1.96 or newer, a C compiler, and `protoc` (the Protocol Buffers compiler, used to build the store's format library):

```sh
xcode-select --install && brew install protobuf                      # macOS
sudo apt install build-essential protobuf-compiler                  # Debian, Ubuntu
sudo dnf install gcc protobuf-compiler                              # RHEL family, Fedora

cargo install --git https://github.com/MBagory/wise-bucket wisebucket --locked
wisebucket --version
```

The build takes several minutes. It puts `wisebucket` and its short alias `wbk` in `~/.cargo/bin`, which rustup already added to your `PATH`. To update, run it again. You also need an MCP-capable agent: Claude Code, OpenAI Codex, Cursor, Kilo Code, Cline…

| | Requirement |
| --- | --- |
| macOS | Apple Silicon or Intel |
| Linux | x86_64 or arm64 |
| Windows | through [WSL2](#windows-wsl2) |
| Disk | room for the store: a decoded copy of your recordings' signals (see [The store and SQL](#the-store-and-sql)) |

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
| `.csv`, `.jsonl` / `.ndjson`, `.parquet`, `.arrow` / `.feather` / `.ipc` | a table |
| `.bag` | a ROS 1 bag: listed, not readable yet |

```sh
wisebucket roots add bags ~/robot-logs/bags --hardware rover-b
wisebucket roots add flights /Volumes/lab-nas/px4 --hardware drone-2   # network mounts work
wisebucket roots list
wisebucket roots check      # readable? overlapping? how many recordings?
wisebucket roots remove flights                                        # the folder itself is never touched
```

**Rules:**
- **Names** are short and stable (`bags`, `lab-nas`): lowercase letters, digits, `-` and `_`, at most 32 characters. Recordings are referenced as `name:relative/path` (`bags:2026-09-24/dock_run3`), so if a mount point moves, you only update that folder's path.
- **Hardware** (optional) names what the recordings come from (a robot, a vehicle, a test bench). It appears in the `recordings` table, so your agent can compare all recordings of one hardware.
- **Include and exclude** (optional) are glob patterns on file paths: `--exclude '**/tmp/**'`. A rosbag2 folder is matched through its `metadata.yaml`.
- **No overlap:** a folder inside another declared folder is refused.
- **No escapes:** symlinks must stay inside their folder.
- **Read-only:** Wise Bucket never modifies, moves or deletes recordings.
- **Your decision only:** Wise Bucket gives your agent no tool to add a folder. You add folders from the command line or a configuration file.
- **Live:** folder changes reach a running agent session on its next call, no restart needed.

**No recordings yet?** `wisebucket demo` downloads public sample files (ROS 2 bags, MCAP, ULog, Parquet, and formats not readable yet) and declares them as the `demo` folder. See [Try it with sample data](../README.md#try-it-with-sample-data).

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

While an agent session is open, Wise Bucket walks your folders at the start, then every minute (and each time the agent calls `server_info`). New and changed recordings are extracted in the background, newest first, one at a time, at low priority. A file changed in the last 5 seconds waits: it may still be being written. Several agent windows share the work and never extract the same file twice.

Each recording has a **status**:

| Status | Meaning |
| --- | --- |
| `done` | Extracted: its streams can be queried |
| `pending` | Waiting to be extracted (only happens while a session is open) |
| `extracting` | Being extracted |
| `failed` | Could not be read; `error` says why. Retried when the file changes |
| `paused` | Waiting for free disk space |
| `missing` | The file is gone; its data stays queryable |
| `unsupported` | A format Wise Bucket cannot read yet (ROS 1 bags) |

### What gets extracted

Every recording becomes one or more **streams**: the topics of a ROS 2 bag or MCAP file, the messages of a ULog file, or a single `table` for CSV, JSON Lines, Parquet and Arrow files. Each stream is a table with:

- **`t`**, a nanosecond timestamp: the receive time for bags and MCAP, `timestamp` for ULog (time since boot, so it shows as a date near 1970), or the time column of a table.
- **`t_sensor`**, the message's own `header.stamp`, when a ROS 2 message has one.
- **`message_bytes`**, the size of each message (bags and MCAP).
- **The message fields**, nested names joined with dots (`linear_acceleration.z`). Arrays stay arrays.

**ROS 2 messages** are decoded from the definitions stored in the recording. ROS 2 Humble bags (`.db3`) don't store them, so standard types (`std_msgs`, `geometry_msgs`, `sensor_msgs`, `nav_msgs`, `diagnostic_msgs`, `trajectory_msgs`, `tf2_msgs`, `/rosout`) are decoded from definitions bundled with Wise Bucket. Other custom types in Humble bags keep their timing and sizes only.

**Tables** need a time column. Wise Bucket uses the first timestamp column, else a column named `time`, `timestamp`, `t` or `stamp` (any case), whose unit comes from a `_s`, `_ms`, `_us` or `_ns` suffix (seconds by default). The stream's `notes` say which column was used. Without one, `t` is empty.

### Querying

Your agent writes the SQL; you can run the same queries with `wisebucket sql`. There are three things to query (full list of columns: [tables reference](reference/tables.md)):

- `recordings`: every recording, with its status, hardware and time range
- `streams`: every stream of every recording, with its row count, columns and notes
- `stream('<name>')`: the rows of one stream across all recordings, with a `recording` column; `stream('<name>', '<recording LIKE pattern>')` restricts the recordings

```sh
wisebucket sql "SELECT status, count(*) FROM recordings GROUP BY status"
wisebucket sql "SELECT recording, stream, rows FROM streams WHERE stream ILIKE '%imu%'"
wisebucket sql "SELECT date_bin(INTERVAL '1 second', t) AS w, count(*) FROM stream('/scan', 'bags:run3%') GROUP BY w ORDER BY w"
```

Queries are read-only `SELECT`s, run by [Apache DataFusion](https://datafusion.apache.org) (a dialect close to PostgreSQL). Each query is limited to 30 seconds, 1 GB of memory and 200 returned rows (`truncated` says when more exist), and sees only the store. Quote dotted and upper-case column names: `"linear_acceleration.z"`. Add `--json` for machine-readable output.

### Opening the store yourself

The store is a set of [Lance](https://github.com/lance-format/lance) datasets, readable from Python, for a notebook for example:

```python
import json, lance
rec = json.load(open("<state directory>/store/<key>/<version>/recording.json"))
imu = next(s for s in rec["streams"] if s["name"] == "/imu")
df = lance.dataset(f"<state directory>/store/<key>/<version>/{imu['dataset']}").to_table().to_pandas()
```

`pip install pylance` provides `lance`; `.to_table()` gives Arrow, which pandas and Polars read directly. Prefer `wisebucket sql` for anything you can express in SQL: it finds the datasets for you.

## Viewers

To see what your agent found, open the original recording in a viewer at the time it points to. These viewers open the same files Wise Bucket reads:

| Viewer | Opens | License | Agent control (MCP) | Get it |
| --- | --- | --- | --- | --- |
| [Rerun](https://rerun.io) | MCAP, including ROS 2 bags recorded as MCAP (experimental) | Open source (MIT, Apache-2.0) | [`rerun viewer-mcp`](https://rerun.io/docs/reference/viewer/mcp) | `pip3 install rerun-sdk` or `cargo binstall rerun-cli` ([all options](https://rerun.io/docs/getting-started/installing-viewer)) |
| [Foxglove](https://foxglove.dev) | MCAP, ROS 2 `.db3`, PX4 ULog | Proprietary, sign-in on first launch, free plan for small teams | [Desktop app MCP server](https://docs.foxglove.dev/docs/agents/mcp-server) (paid plan) | [Download](https://foxglove.dev/download) |
| [Lichtblick](https://github.com/lichtblick-suite/lichtblick) | MCAP, PX4 ULog | Open source (MPL-2.0) | None | [Web app](https://lichtblick-suite.github.io/lichtblick/) or [desktop releases](https://github.com/lichtblick-suite/lichtblick/releases) |
| [PlotJuggler](https://github.com/PlotJuggler/PlotJuggler) | MCAP, ROS 2 bags, PX4 ULog, CSV, Parquet | Open source (MPL-2.0) | None | [Releases](https://github.com/PlotJuggler/PlotJuggler/releases), `sudo snap install plotjuggler`, or `sudo apt install ros-$ROS_DISTRO-plotjuggler-ros` |

With a viewer's MCP server connected to the same agent, the agent can open the file and move to the right time itself. PlotJuggler's real site is its GitHub page: its README warns that `plotjuggler.com` is not theirs.

## Everyday use

| You want to… | Run |
| --- | --- |
| Check that everything works | `wisebucket doctor` |
| See what has been extracted | `wisebucket sql "SELECT status, count(*) FROM recordings GROUP BY status"` |
| See why a recording failed | `wisebucket sql "SELECT recording, error FROM recordings WHERE status = 'failed'"` |
| See your settings and where each one comes from | `wisebucket config show --origin` |

- **Several windows at once:** each agent window starts its own Wise Bucket process. They share the store and the extraction work.
- **Machine-readable output:** `config`, `roots`, `sql` and `doctor` accept `--json`.

## Where things live

| What | Where |
| --- | --- |
| Your settings and recording folders | Linux `~/.config/wisebucket/config.toml`, macOS `~/Library/Application Support/wisebucket/config.toml` |
| How your agent starts Wise Bucket | Your agent's user-level MCP settings (e.g. `claude mcp get wise-bucket`) |
| State directory | Linux `~/.local/share/wisebucket/`, macOS `~/Library/Application Support/wisebucket/`. To use another disk (the store grows with your recordings), set `state_dir = "…"` in your settings |

```text
<state directory>/
├── store/            the decoded copy of your recordings (derived; safe to delete)
│   ├── candidates.json        the recordings found in your folders
│   └── <key>/<version>/       one recording: recording.json, data/NNNN.lance (one dataset per stream)
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

The next agent session extracts everything again. Your recordings and settings are untouched.

## Uninstall

```sh
claude mcp remove --scope user wise-bucket    # or remove the entry from your agent's MCP settings
cargo uninstall wisebucket                    # removes wisebucket and wbk
```

Your store stays in the state directory (`wisebucket config path` shows where). Delete it yourself if you no longer need it; Wise Bucket never deletes it. On macOS your settings live in the same folder, so deleting it removes them too. Your recordings are never touched.
