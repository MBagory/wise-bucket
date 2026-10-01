# Troubleshooting and FAQ

**Start with `wise-bucket-server doctor`.** It checks your configuration, the database, your recording folders and your agent's configuration. Each problem comes with an error code and a link to its fix in the [error reference](reference/errors.md). Add `-v` to any command for diagnostic logs. The MCP server logs to `<state directory>/logs/server.log`.

- [Troubleshooting](#troubleshooting)
- [FAQ](#faq)

## Troubleshooting

### `wise-bucket` doesn't appear in my agent

- Start the agent **in the repository** that contains `.mcp.json` (or your agent's equivalent), and approve the project server when asked. In Claude Code, `/mcp` lists it.
- Check that the `command` path in `.mcp.json` exists; `doctor` reports a moved or deleted binary. After reinstalling, run `wise-bucket-server init --merge`.
- Run the command from `.mcp.json` by hand. It should wait silently for input (press Ctrl-C to quit). If it prints an error instead, that error tells you what is wrong.

### The agent says the database is not set up (`db_not_initialized`)

Run `wise-bucket-server setup` once on this machine. If you use `--state-dir` or `WB_STATE_DIR`, use the same value for `setup` and in `.mcp.json`; `init` records it automatically.

### Setup fails with `toolchain_missing` or `pgvector_build_failed`

Install the C toolchain (see [Install](guide.md#install)) and re-run `setup`. The error message gives the path of the build log. On macOS, if Xcode was updated, run `xcode-select --install` again.

### Setup fails with `download_failed` or `checksum_mismatch`

- `download_failed`: check network access to `github.com` (proxy, firewall), then re-run `setup`. Completed downloads are kept.
- `checksum_mismatch`: the downloaded file is corrupted or was tampered with. Delete `<state directory>/runtime/downloads` and try again. If it happens again, please report it (see [SECURITY.md](../SECURITY.md)).

### The database doesn't start (`db_start_failed`)

Look at `<state directory>/logs/postgres.log`. The usual causes:
- **The disk is full:** free some space. `doctor` shows how much is left.
- **A crash left a lock file:** after a power loss, a stale `postmaster.pid` can remain in `<state directory>/pg`. PostgreSQL normally cleans it up. If it doesn't, make sure no `postgres` process is running for that directory, then delete the file.
- **The socket path is too long** (`socket_path_too_long`): use a shorter `--state-dir`.

### A recording folder is reported invalid, overlapping or outside the project

- `root_invalid`: the folder doesn't exist, isn't readable, or its name isn't valid. `wise-bucket-server roots check` shows the details.
- `root_overlap`: one folder contains another. Keep the outer one, or split them.
- `root_outside_project`: folders in `.wisebucket/config.toml` must be relative paths inside the repository. Declare other folders in your user configuration with `roots add` (without `--in-project`).

Restart the agent session after changing folders.


### WSL2

- Run the agent **inside WSL** so it can start the Linux binary.
- Folders under `/mnt/c/...` work but are slow to scan; for large recordings, prefer the WSL file system.
- If PostgreSQL doesn't start after a Windows reboot, run `wise-bucket-server db status` and check `postgres.log`. A stale lock file is the usual cause.

### Still stuck?

Open an issue with the output of `wise-bucket-server doctor --json`; it contains no secrets.

## FAQ

### Does Wise Bucket call an LLM or use my API key?

No. Wise Bucket never calls a model. Your agent owns the model, your key and your token budget.

### Do my recordings leave my machine?

No. Wise Bucket reads them where they are. Only the tool results your agent sends to its model leave your machine, under your agent's terms; see [SECURITY.md](../SECURITY.md#privacy).

### Why PostgreSQL? Do I have to install it?

PostgreSQL with pgvector gives Wise Bucket reliable storage that several agent windows can share, plus vector and full-text search. You don't install it yourself: `setup` downloads a private copy, verifies it, and Wise Bucket starts and stops it as needed. It's private to Wise Bucket: it listens only on a Unix socket, so it never conflicts with another PostgreSQL on the machine.

### Is PostgreSQL always running in the background?

No. It starts with your first agent session, in about 2 seconds, and stops when the last one ends. To keep it running, set `keep_running = true` under `[database]` in your user configuration, or `WB_KEEP_RUNNING=true`.

### Why does setup need a C compiler?

pgvector is distributed as source code. `setup` builds the pinned, checksum-verified release once, in about 15 seconds, against the exact PostgreSQL it downloaded.

### Can my agent read files outside my recording folders through Wise Bucket?

No. Wise Bucket only reads inside the folders you declared, refuses symlinks that escape them, and has no tool that adds a folder. Your agent's own tools, a shell for example, are outside Wise Bucket's control; restrict them in the agent if you need to.

### Can I have several recording folders, or folders on a NAS?

Yes. Declare as many as you like with `wise-bucket-server roots add <name> <path>`, network mounts included. Folders can't overlap.

### Can I use several agent windows at the same time?

Yes. Each window starts its own Wise Bucket process, and they share the same database.

### Where is my data, and how do I back it up?

`wise-bucket-server config path` shows the locations. To back up, run `wise-bucket-server backup wb.dump`; to restore, run `restore wb.dump --yes`. Your recordings are never part of a backup: they stay where they are.
