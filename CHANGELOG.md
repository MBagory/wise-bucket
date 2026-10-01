# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added: sample data

- `demo [FORMAT…] | --all | --list`: downloads public sample recordings (ROS 2 MCAP and SQLite bags, MCAP, ROS 1 bags, PX4 ULog, ArduPilot DataFlash, MAVLink tlog, CAN ASC/BLF + DBC, MDF4, Parquet) from their upstream projects at pinned commits, verifies their SHA-256, and declares them as the `demo` data root. Nothing is redistributed by this repository.
- Per-format sample tests (`cargo test -p wb-server --test samples -- --ignored`), run by a dedicated CI job per format.

### Added: milestone M0 (foundation)

- `wise-bucket-server` binary: MCP server over stdio (`serve`) and command-line tool.
- `setup`: provisions a managed PostgreSQL 17.11 + pgvector 0.8.6 (pinned, SHA-256 verified, pgvector built locally against the downloaded PostgreSQL), or checks your own PostgreSQL (`--database-url` / `WB_DATABASE_URL`); interactive or `--yes`.
- Data roots: `roots add|list|remove|check`, user and project (`--in-project`) roots, overlap/symlink/containment validation, candidate-recording counts.
- `init`: writes `.wisebucket/config.toml` and the harness MCP configuration (Claude Code `.mcp.json`, Kilo Code `.kilocode/mcp.json`, Cline snippet); never overwrites, `--merge` to add to an existing file.
- Layered configuration (flags > `WB_*` env > project > user > defaults) with `config show --origin` and `config path`.
- Managed database lifecycle: starts with the first MCP session, stops after the last one (`keep_running` to disable); `db status|start|stop`.
- MCP tool `server_info`: versions, database status, session, project, default robot, data roots and root problems.
- Sessions recorded in the database (client name/version from MCP `initialize`, heartbeat, end).
- `backup` / `restore` (pg_dump custom format, Wise Bucket schemas only).
- `doctor`: checks configuration, state directory, database, roots and harness configuration, with error codes and documentation links.
- Structured errors with stable codes and documented fixes; generated reference pages (`docs gen`).
- Documentation as plain Markdown in `docs/`: user guide, troubleshooting and FAQ, generated references (CLI, configuration, errors), example configurations; privacy notes in `SECURITY.md`.
