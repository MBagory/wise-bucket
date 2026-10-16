# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-10-16

Wise Bucket 0.2.0 reads the logs of vehicles, test benches, drones and boats:

- **Vehicles and test benches:** CAN logs from Vector tools (ASC, BLF) and MDF4 measurement files. The first time, the agent asks you which DBC file decodes your CAN logs. From then on, every log in that folder is decoded into signals, and the raw frames stay available next to them.
- **Drones:** ArduPilot logs, alongside PX4 and MAVLink logs.
- **Boats:** NMEA 0183 and NMEA 2000 logs.
- **Robots:** MCAP files with protobuf or JSON messages, as Foxglove tools record them.

### Added

- **CAN logs:** Vector ASC and BLF logs (CANoe, CANalyzer) are extracted into one stream, `frames`: the channel as the log numbers it, the identifier, the extended, CAN FD, remote and error flags, the direction, the DLC and the payload bytes. ASC times count from the start of the measurement, whose date is in `properties`; BLF times start at the file's start time, read as UTC. Error frames and lines that are not frames (statistics, J1939 transport) are counted in `recordings.notes`, and a BLF file cut while logging keeps its valid part.
- **CAN signals from your DBC files:** the first time a CAN log has no DBC decision, its notes tell the agent to ask you: to confirm the `.dbc` found next to the log, to choose among several (and the channel for each), or where it is. The answer is recorded with `note` (key `dbc`, or `none`) on the folder or root it holds for, so later logs are decoded without asking. Each DBC message becomes a stream `<dbc>/<message>` of physical values, with units in the stream notes and value-table labels; `frames` stays, and frames no DBC decodes are counted in `recordings.notes`. Editing a DBC or the answer extracts the logs again ([ADR 0029](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0029-can-signals-from-confirmed-dbc.md)).
- **ArduPilot DataFlash logs** (`.bin`): one stream per message type (`ATT`, `IMU`, `GPS`…), scaled as pymavlink reads them, with the log's units in `streams.notes`; `PARM` (parameters), `MSG`, `ERR` and `EV`; the firmware and the `VER` record in `properties`; error codes counted in `recordings.notes`; and the start time in UTC when the log has a GPS fix.
- **MCAP protobuf and JSON messages** are decoded: protobuf with the schema the file stores, JSON as it is; one column per field, nested messages as structs, enums by name, and `bytes` fields as their length, so images and point clouds keep their sizes only.
- **NMEA 0183 logs** (`.nmea`): every sentence in the stream `sentences`, as written, with its talker, type and checksum validity, so proprietary sentences stay queryable; the sentence types the `nmea` crate decodes also get one stream each (`GGA`, `RMC`, `GSV`…). Times are the sentences' UTC times on the date of the latest `RMC` or `ZDA`. Bad checksums and the sentence types not decoded are counted in `recordings.notes`.
- **NMEA 2000 captures** (`.raw`, canboat's PLAIN format): every frame in the stream `frames`, and one stream per message decoded with the CANboat PGN database (`windData`, `vesselHeading`…), fast-packet messages reassembled, values in SI units with each stream's units in its notes. The PGNs whose frames were not decoded are listed in `recordings.notes`.
- **MDF4 files** (`.mf4`): one stream per channel group, with physical values and units, timed from the file's start time and the group's time master; the header's comment and properties go to `properties`.

### Changed

- Building from source needs Rust 1.98.

## [0.1.1] - 2026-10-16

### Changed

- The release archives are `.tar.gz`, so the installer works on Linux systems without `xz`, such as minimal and container images ([ADR 0028](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0028-release-archives.md)).
- `wbk` is installed as a link to `wisebucket`: the download holds one binary, and the installation takes half the space.

## [0.1.0] - 2026-10-15

### Added

- **Every recording in your data roots is extracted into a store**: a decoded, columnar copy of its signals, one table per stream, with a common nanosecond time column `t` (plus `t_sensor` from `header.stamp`, and `message_bytes`). Wise Bucket trades disk space for query speed ([ADR 0012](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0012-trade-storage-for-query-speed.md)). Readable formats: ROS 2 bags (MCAP or SQLite `.db3`; Humble bags are decoded with bundled standard message definitions), MCAP files, PX4 ULog (with its parameters and their defaults, logged messages, logger dropouts, texts such as the boot console or a crash dump, and the message types the file defines (`formats`, with those never logged) as streams, its info in `properties`, and its start from the header), MAVLink telemetry logs (`.tlog`, one stream per message, with `system_id` and `component_id`), CSV, JSON Lines, Parquet and Arrow IPC. A format ships only when an external Rust crate decodes it ([ADR 0016](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0016-formats-need-an-external-rust-decoder.md)). ROS 2 topics keep their publishers' QoS in short (`streams.qos`: `reliable, volatile, depth 10`). Images, point clouds, maps and video keep their timing and sizes only. A stream whose time never changes is flagged in its `notes`. ROS 1 bags, ArduPilot DataFlash logs, CAN ASC and BLF logs, and MDF4 files are listed as `unsupported`; `.bin` and `.asc` files only when their first bytes match.
- **Automatic, in the background:** while an agent session is open, new and changed recordings are found through file events and extracted, newest first, one at a time, in a low-priority child process: a file renamed into place (a finished download) at once, a file written in place once it has not changed for 5 seconds ([ADR 0018](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0018-extraction-on-file-events.md)). Several agent windows never extract the same file twice. A deleted recording stays queryable as `missing`. Extraction pauses below 10 GB of free disk space.
- **Notes that point to problems:** `recordings.notes` counts the warnings and errors logged by ULog, ROS 2 `/rosout` and MAVLink `STATUSTEXT`, and quotes the first one; it also reports ULog logger dropouts, crash dumps and files cut or damaged. Stream notes flag ULog timestamps that are not set.
- **The `sql` tool** (and `wisebucket sql "<query>"`): read-only SQL over the tables `recordings`, `streams` and `properties` (short facts a recording states about itself, such as MCAP metadata records) and the function `stream('<name>' [, '<recording LIKE pattern>'])`, run by Apache DataFusion over Parquet files, with no database to install ([ADR 0011](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0011-no-database.md)). Nothing is computed in advance: rates, gaps, windows, percentiles and joins are written when a question needs them ([ADR 0010](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0010-metrics-are-sql-written-by-the-agent.md)). Queries are capped at 30 s and 1 GB; DDL, writes and file access are refused. A result holds about 60 KB of rows (or `limit` rows, `--limit` in the terminal), as arrays in the order of `columns`, with their `types`; when rows are cut, a `hint` says how to aggregate or fetch the next page, by another column when the last rows share one time. Recipes show how to plot (min, avg and max per time bin) and how to page by time window or by key ([ADR 0017](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0017-result-size-and-paging.md)).
- **What you know about recordings:** the `note` tool (and `wisebucket note <target> key=value…`) records what the engineer says that the files don't, for a recording, a folder or a root; it shows at once in `recordings.context` (a map: `context['hardware']` is the machine), where the most specific target wins. It is kept in `context.toml`, next to the settings file, and never in the recordings.
- **The `rerun` tool** (and `wisebucket rerun "<query>"`): sends every row of a read-only query to a running Rerun viewer, with no size cap ([ADR 0019](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0019-send-query-results-to-rerun.md)). Each timestamp column (`t`, `t_sensor`) becomes a timeline, shown as durations when it counts from boot; each numeric or boolean column a scalar series at `<entity>/<column>`. All sends go to the Rerun recording `wisebucket`.
- **Abnormal and similar recordings:** recipes for a robust z-score of one value per recording against its peers, spikes inside a recording, and the recordings closest to a reference by distribution ([ADR 0020](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0020-abnormality-and-similarity-as-sql-recipes.md)).
- **The `server_info` tool:** the version, the store (recordings per status, size, paused), and each data root with its candidate recordings by kind (each root's scan stops after 5 seconds). Declared read-only.
- **Prebuilt binaries** for macOS (Apple Silicon and Intel) and Linux (x86_64 and arm64, static, for any distribution), and a shell installer that puts them in `~/.cargo/bin`.
- **The `wisebucket` binary** (short alias `wbk`): the MCP server over stdio (`serve`) and the command-line tool, built on DataFusion and Parquet for instant analysis.
- `setup`: asks for your recording folders (or `--yes --root name=path`), downloads nothing, and prints how to register Wise Bucket once for your user (`claude mcp add --scope user …`, or a JSON entry for other agents).
- Data roots, user-wide ([ADR 0004](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/0004-user-wide-only.md)): `roots add|list|remove|check`, include and exclude globs (a rosbag2 folder is matched through its `metadata.yaml`), overlap and symlink checks, and candidate recordings counted by kind.
- Configuration: flags > `WB_*` environment variables > user configuration > defaults, with `config show --origin` and `config path`.
- `doctor`: checks the configuration, the state directory, the store, SQL and the data roots, with error codes and documentation links.
- `demo [FORMAT…] | --all | --list`: downloads public sample recordings (ROS 2 MCAP and SQLite bags, MCAP, PX4 ULog, ArduPilot DataFlash, MAVLink tlog, CAN ASC/BLF, MDF4, Parquet; six PX4 ULogs, to compare recordings; only formats an existing Rust crate can decode) from their upstream projects at pinned commits, verifies their SHA-256, and declares them as the `demo` data root. Nothing is redistributed by this repository. Downloads time out (30 s to connect, 60 s for the server to answer, 30 min per file) with `download_failed`.
- `completions <bash|zsh|fish|elvish|powershell>` prints a completion script for the name it is invoked as (`wisebucket` or `wbk`).
- Structured errors with stable codes and documented fixes; JSON errors, in tool results and `--json` output, include the absolute `docs_url`.
- CLI: colored status glyphs and `--help` (off when piped or with `NO_COLOR`), examples in `--help`, `--json` on `config`, `roots`, `sql` and `doctor`, with errors printed as `{"error": {…}}` on stderr.
- Documentation as plain Markdown: the README with Wise Bucket's principles, the user guide (with compatible [viewers](https://github.com/MBagory/wise-bucket/blob/main/docs/guide.md#viewers)), troubleshooting and FAQ, generated references (CLI, configuration, errors, tables), [decision records](https://github.com/MBagory/wise-bucket/blob/main/docs/decisions/README.md) with the list of possible features that replaces a roadmap, and privacy notes in `SECURITY.md`.
- Tests: acceptance tests drive the real binary through a real MCP client; format tests check exact values on generated fixtures; per-format sample tests (`cargo test -p wisebucket --test samples -- --ignored`) run in a dedicated CI job per format.

[Unreleased]: https://github.com/MBagory/wise-bucket/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/MBagory/wise-bucket/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/MBagory/wise-bucket/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MBagory/wise-bucket/releases/tag/v0.1.0
