# Troubleshooting

**Start with `wise-bucket-server doctor`.** It checks configuration, database, data roots and harness setup, and prints an error code and a documentation link for each problem. Every error code is explained in the [error reference](reference/errors.md). Add `-v` to any command for diagnostic logs; the MCP server logs to `<state directory>/logs/server.log`.

## `wise-bucket` is not listed in Claude Code's `/mcp`

- Make sure you started Claude Code **in the repository** that contains `.mcp.json`, and that you approved the project server when asked.
- Check that the `command` path in `.mcp.json` exists: `doctor` reports a moved or deleted binary. Re-run `wise-bucket-server init --merge` after reinstalling.
- Run the command from `.mcp.json` by hand: it should wait silently for input (press Ctrl-C to quit). An immediate error message tells you what is wrong.

## `server_info` reports `db_not_initialized`

Run `wise-bucket-server setup` once on this machine. If you use `--state-dir` or `WB_STATE_DIR`, use the same value for `setup` and in `.mcp.json` (`init` records it automatically).

## Setup fails with `toolchain_missing` or `pgvector_build_failed`

Install the C toolchain (see [Install](get-started/install.md#2-install-the-c-toolchain-once)) and re-run `setup`. The build log path is printed in the error message. On macOS, if Xcode was updated, run `xcode-select --install` again.

## Setup fails with `download_failed` or `checksum_mismatch`

- `download_failed`: check network access to `github.com` (proxy, firewall), then re-run `setup`; completed downloads are kept.
- `checksum_mismatch`: the downloaded file is corrupted or was tampered with. Delete `<state directory>/runtime/downloads` and retry. If it happens again, please report it (see SECURITY.md).

## The database does not start (`db_start_failed`)

Look at `<state directory>/logs/postgres.log`. Common causes:

- **Disk full:** free some space; `doctor` shows free space.
- **Crash leftovers:** a stale `postmaster.pid` in `<state directory>/pg` after a power loss. PostgreSQL normally cleans it up; if not, make sure no `postgres` process is running for that directory, then delete the file.
- **Socket directory:** `socket_path_too_long` means the state directory path is too long for a Unix socket. Use a shorter `--state-dir`.

## A data root is reported invalid, overlapping or outside the project

- `root_invalid`: the folder does not exist, is not readable, or the name is not valid. `wise-bucket-server roots check` shows details.
- `root_overlap`: one root contains another. Keep the outer one or split them.
- `root_outside_project`: project roots (in `.wisebucket/config.toml`) must be relative paths inside the repository. Put other folders in your user config with `roots add` (without `--in-project`).

Remember to restart the harness session after changing roots.

## Using your own PostgreSQL

- `vector_extension_missing`: install pgvector 0.8 or newer on the server and run `CREATE EXTENSION vector;` as a superuser in the target database. `pg_trgm` is part of PostgreSQL's contrib package.
- `db_privileges_insufficient`: the user must own the database so migrations can create the `wb` and `wb_views` schemas.
- `db_version_unsupported`: PostgreSQL 15 or newer is required.

## WSL2 specifics

- Run the harness **inside WSL** so it can start the Linux binary.
- Roots under `/mnt/c/...` work but are slow to scan; prefer the WSL file system for large recordings.
- If PostgreSQL does not start after a Windows reboot, run `wise-bucket-server db status` and check `postgres.log`; a stale lock file is the usual cause.

## Still stuck?

Open an issue with the output of `wise-bucket-server doctor --json` (it contains no secrets).
