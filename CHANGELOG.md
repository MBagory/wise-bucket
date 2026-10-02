# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Wise Bucket is now **user-wide only**: data is organized by recording folder, not by repository. `init`, the project configuration (`.wisebucket/config.toml`), the global `--project` flag, `WB_PROJECT_DIR` and `roots add|remove --in-project` are removed, as are the `root_outside_project` and `mcp_config_exists` error codes. `setup` now ends by printing how to register the server once for your user (`claude mcp add --scope user …`, or the JSON entry for other agents). Existing databases must be recreated (the `session.project` column is gone from the initial schema).
- The binary is renamed from `wise-bucket-server` to `wisebucket`, with a short alias `wbk` (not `wb`, which Weights & Biases installs).

### Fixed

- Managed `setup` no longer fails with `pgvector_build_failed` when the state or runtime directory contains a space (the default on macOS: `~/Library/Application Support`). pgvector is now built from a space-free scratch directory under `/tmp`.

### Added: sample data

- `demo [FORMAT…] | --all | --list`: downloads public sample recordings (ROS 2 MCAP and SQLite bags, MCAP, ROS 1 bags, PX4 ULog, ArduPilot DataFlash, MAVLink tlog, CAN ASC/BLF + DBC, MDF4, Parquet) from their upstream projects at pinned commits, verifies their SHA-256, and declares them as the `demo` data root. Nothing is redistributed by this repository.
- Per-format sample tests (`cargo test -p wb-server --test samples -- --ignored`), run by a dedicated CI job per format.

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
