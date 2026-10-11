# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added: compatible viewers in the guide

- [Guide: viewers](docs/guide.md#viewers): Rerun, Foxglove, Lichtblick and PlotJuggler, what each one opens, their MCP servers, and where to get them.

### Added: the store, and SQL for your agent (milestone M1)

- **Every recording in your data roots is extracted into a store**: a decoded, columnar copy of its signals, one table per stream, with a common nanosecond time column `t` (plus `t_sensor` from `header.stamp`, and `message_bytes`). Wise Bucket trades disk space for query speed ([ADR 0012](docs/decisions/0012-trade-storage-for-query-speed.md)). Readable formats: ROS 2 bags (MCAP or SQLite `.db3`; Humble bags are decoded with bundled standard message definitions), MCAP files, PX4 ULog, MAVLink telemetry logs (`.tlog`, one stream per message, with `system_id` and `component_id`), CSV, JSON Lines, Parquet and Arrow IPC. Images, point clouds, maps and video keep their timing and sizes only. ROS 1 bags are listed as `unsupported`.
- **Automatic, in the background:** while an agent session is open, new and changed recordings are found every minute and extracted, newest first, one at a time, in a low-priority child process. Several agent windows never extract the same file twice. A deleted recording stays queryable as `missing`. Extraction pauses below 10 GB of free disk space.
- **The `sql` tool** (and `wisebucket sql "<query>"`): read-only SQL over the tables `recordings` and `streams` and the function `stream('<name>' [, '<recording LIKE pattern>'])`, run by Apache DataFusion over Lance datasets. Nothing is computed in advance: rates, gaps, windows, percentiles and joins are written when a question needs them ([ADR 0010](docs/decisions/0010-metrics-are-sql-written-by-the-agent.md)). Queries are capped at 30 s and 1 GB; DDL, writes and file access are refused. A result holds about 60 KB of rows (or `limit` rows, `--limit` in the terminal), as arrays in the order of `columns`; when rows are cut, a `hint` says how to aggregate or fetch the next page. Recipes show how to plot (min, avg and max per time bin) and how to page by time window or by key ([ADR 0017](docs/decisions/0017-result-size-and-paging.md)).
- `server_info` reports the store (recordings per status, size, paused) and each root's hardware; `doctor` checks the store and SQL.
- References: [tables](docs/reference/tables.md) (generated); [decision records](docs/decisions/README.md) for why Wise Bucket is built this way, and the list of possible features that replaces the milestone roadmap.
- New error codes: `recording_unreadable`, `query_failed`.

### Removed: PostgreSQL

- The managed PostgreSQL is gone ([ADR 0011](docs/decisions/0011-no-database.md)): `setup` downloads nothing, and nothing runs in the background without an agent session. Removed: `db status|start|stop`, `backup`, `restore`, `--keep-running`, `WB_KEEP_RUNNING`, `WB_RUNTIME_DIR`, and the error codes `unsupported_platform`, `db_not_initialized`, `db_start_failed`, `db_connect_failed`, `migration_failed`, `socket_path_too_long` and `backup_failed`. The platform limits it imposed (macOS 15+, glibc 2.34+) are gone.
- The `[database]` section of existing user configurations is ignored; you can delete it. In the state directory, `runtime/`, `pg/`, `secrets/` and `run/` can be deleted.

### Changed

- **Data roots name their hardware, not their robot:** `roots add --hardware` replaces `--robot`, and the configuration key `robot = "…"` becomes `hardware = "…"`. Edit existing lines: the old key fails with `config_invalid`.
- A root's `include`/`exclude` globs are now applied (matched against file paths; a rosbag2 folder through its `metadata.yaml`).
- `roots check` and `server_info` count recordings by kind: `rosbag2` replaces `rosbag2_dirs`, and `db3`, `csv`, `json_lines`, `parquet`, `arrow_ipc` and `tlog` are new.
- Building from source needs Rust 1.96 or newer, a C compiler and `protoc`.

### Removed: pgvector until M4, and the 0.1.0-rc.1 pre-release

- The 0.1.0-rc.1 pre-release is withdrawn. Binaries will be published from the first usable version (milestone M4); until then, install from source (`cargo install --git https://github.com/MBagory/wise-bucket wisebucket --locked`).
- `setup` no longer installs pgvector, and the database no longer creates the `vector` extension: nothing uses it before similarity search arrives in M4. `server_info`, `doctor` and `db status` no longer report it, and the `vector_extension_missing` error code is removed. Existing databases must be recreated (see [Reset](docs/guide.md#reset)).

### Added: prebuilt binaries and installer

- Install with `curl -LsSf https://github.com/MBagory/wise-bucket/releases/latest/download/wisebucket-installer.sh | sh`. You no longer need Rust. Releases ship static binaries for macOS (Apple Silicon, Intel) and Linux (x86_64, arm64), built by [dist](https://github.com/axodotdev/cargo-dist) when a `v*` tag is pushed.
- `setup` downloads a prebuilt, checksum-pinned pgvector instead of compiling it, so a C compiler is no longer needed. The `toolchain_missing` and `pgvector_build_failed` error codes are removed.
- The Cargo package is renamed from `wb-server` to `wisebucket` (`cargo run -p wisebucket`, `cargo test -p wisebucket`).
- Documented platform limits, set by the managed PostgreSQL binaries: macOS 15 or later; Linux with glibc 2.34 or later (Ubuntu 22.04+, Debian 12+, RHEL 9+).

### Changed

- Wise Bucket is now **user-wide only**: data is organized by recording folder, not by repository. `init`, the project configuration (`.wisebucket/config.toml`), the global `--project` flag, `WB_PROJECT_DIR` and `roots add|remove --in-project` are removed, as are the `root_outside_project` and `mcp_config_exists` error codes. `setup` now ends by printing how to register the server once for your user (`claude mcp add --scope user …`, or the JSON entry for other agents). Existing databases must be recreated (the `session.project` column is gone from the initial schema).
- The binary is renamed from `wise-bucket-server` to `wisebucket`, with a short alias `wbk` (not `wb`, which Weights & Biases installs).

- `server_info` declares itself read-only (MCP tool annotations), and its error objects no longer repeat `status`. JSON errors, in tool results and `--json` output, now include the absolute `docs_url` next to `docs_ref`.
- `server_info` reports how many candidate recordings each data root holds, by kind (`candidate_recordings`, `candidates`), so the agent can answer *"Which recordings does Wise Bucket see?"*. Each root's scan stops after 5 seconds.

- CLI polish: colored status glyphs and `--help` (off when piped or with `NO_COLOR`), examples in `--help`, `--json` errors printed as `{"error": {…}}` on stderr, `roots check` results on stdout, `restore` asks for confirmation on a terminal (still requires `--yes` otherwise), and `docs gen` is hidden from help (contributor tool).

### Added: shell completions

- `completions <bash|zsh|fish|elvish|powershell>` prints a completion script for the name it is invoked as (`wisebucket` or `wbk`).

### Fixed

- The agent now gives the full documentation link for an error (`docs_url`), not a relative path.
- Downloads (`setup`, `demo`) no longer hang forever on a dead or stalled connection: they fail with `download_failed` after a timeout (30 s to connect, 60 s for the server to answer, 30 min per file).
- Managed `setup` no longer fails with `pgvector_build_failed` when the state or runtime directory contains a space (the default on macOS: `~/Library/Application Support`). pgvector is now built from a space-free scratch directory under `/tmp`.

### Added: sample data

- `demo [FORMAT…] | --all | --list`: downloads public sample recordings (ROS 2 MCAP and SQLite bags, MCAP, PX4 ULog, ArduPilot DataFlash, MAVLink tlog, CAN ASC/BLF, MDF4, Parquet; only formats an existing Rust crate can decode) from their upstream projects at pinned commits, verifies their SHA-256, and declares them as the `demo` data root. Nothing is redistributed by this repository.
- Per-format sample tests (`cargo test -p wisebucket --test samples -- --ignored`), run by a dedicated CI job per format.

### Added: milestone M0 (foundation)

- `wisebucket` binary: MCP server over stdio (`serve`) and command-line tool.
- `setup`: provisions a managed PostgreSQL 17.11 + pgvector 0.8.6 (pinned, SHA-256 verified, pgvector built locally against the downloaded PostgreSQL), then asks for recording folders (or `--yes`). The managed database is the only database mode.
- Data roots: `roots add|list|remove|check`, overlap/symlink validation, candidate-recording counts.
- Layered configuration (flags > `WB_*` env > project > user > defaults) with `config show --origin` and `config path`.
- Managed database lifecycle: starts with the first MCP session, stops after the last one (`keep_running` to disable); `db status|start|stop`.
- MCP tool `server_info`: versions, database status, session, project, default robot, data roots and root problems.
- Sessions recorded in the database (client name/version from MCP `initialize`, heartbeat, end).
- `backup` / `restore` (pg_dump custom format, Wise Bucket schemas only).
- `doctor`: checks configuration, state directory, database, roots and harness configuration, with error codes and documentation links.
- Structured errors with stable codes and documented fixes; generated reference pages (`docs gen`).
- Documentation as plain Markdown in `docs/`: user guide, troubleshooting and FAQ, generated references (CLI, configuration, errors), example configurations; privacy notes in `SECURITY.md`.
