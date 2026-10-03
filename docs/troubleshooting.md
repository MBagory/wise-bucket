# Troubleshooting and FAQ

**Start with `wisebucket doctor`.** It checks your configuration, the database, your recording folders and your agent's configuration. Each problem comes with an error code and a link to its fix in the [error reference](reference/errors.md). Add `-v` to any command for diagnostic logs. The MCP server logs to `<state directory>/logs/server.log`.

- [Troubleshooting](#troubleshooting)
- [FAQ](#faq)

## Troubleshooting

### `wise-bucket` doesn't appear in my agent

- Register it once for your user with the command printed at the end of `wisebucket setup`, then start a **new** agent session. In Claude Code, `claude mcp get wise-bucket` shows the entry and `/mcp` lists it.
- Check that the entry's `command` path exists. After reinstalling or moving the binary, re-run `setup` and register the printed entry again (in Claude Code: `claude mcp remove --scope user wise-bucket` first).
- Run the entry's command by hand. It should wait silently for input (press Ctrl-C to quit). If it prints an error instead, that error tells you what is wrong.

### The agent says the database is not set up (`db_not_initialized`)

Run `wisebucket setup` once on this machine. If you use `--state-dir` or `WB_STATE_DIR`, use the same value for `setup` and in your agent's entry; the entry printed by `setup` includes it.

### Setup fails with `error while loading shared libraries` (Linux)

The managed PostgreSQL needs a few common system libraries that minimal containers can lack. Install the ones listed in [Install](guide.md#install) and re-run `setup`. A `GLIBC_2.34 not found` error means the distribution is too old: see the supported list there.

### Setup fails with `download_failed` or `checksum_mismatch`

- `download_failed`: check network access to `github.com` (proxy, firewall), then re-run `setup`. Completed downloads are kept.
- `checksum_mismatch`: the downloaded file is corrupted or was tampered with. Delete `<state directory>/runtime/downloads` and try again. If it happens again, please report it (see [SECURITY.md](../SECURITY.md)).

### The database doesn't start (`db_start_failed`)

Look at `<state directory>/logs/postgres.log`. The usual causes:
- **The disk is full:** free some space. `doctor` shows how much is left.
- **A crash left a lock file:** after a power loss, a stale `postmaster.pid` can remain in `<state directory>/pg`. PostgreSQL normally cleans it up. If it doesn't, make sure no `postgres` process is running for that directory, then delete the file.
- **The socket path is too long** (`socket_path_too_long`): use a shorter `--state-dir`.

### A recording folder is reported invalid or overlapping

- `root_invalid`: the folder doesn't exist, isn't readable, or its name isn't valid. `wisebucket roots check` shows the details.
- `root_overlap`: one folder contains another. Keep the outer one, or split them.


### WSL2

- Run the agent **inside WSL** so it can start the Linux binary.
- Folders under `/mnt/c/...` work but are slow to scan; for large recordings, prefer the WSL file system.
- If PostgreSQL doesn't start after a Windows reboot, run `wisebucket db status` and check `postgres.log`. A stale lock file is the usual cause.

### Still stuck?

Open an issue with the output of `wisebucket doctor --json`; it contains no secrets.

## FAQ

### Does Wise Bucket call an LLM or use my API key?

No. Wise Bucket never calls a model. Your agent owns the model, your key and your token budget.

### Do my recordings leave my machine?

No. Wise Bucket reads them where they are. Only the tool results your agent sends to its model leave your machine, under your agent's terms; see [SECURITY.md](../SECURITY.md#privacy).

### Why PostgreSQL? Do I have to install it?

PostgreSQL gives Wise Bucket reliable storage that several agent windows can share, plus full-text and fuzzy (trigram) search. Similarity search (the pgvector extension) arrives with milestone M4. You don't install it yourself: `setup` downloads a private copy, verifies it, and Wise Bucket starts and stops it as needed. It's private to Wise Bucket: it listens only on a Unix socket, so it never conflicts with another PostgreSQL on the machine.

### Is PostgreSQL always running in the background?

No. It starts with your first agent session, in about 2 seconds, and stops when the last one ends. To keep it running, set `keep_running = true` under `[database]` in your user configuration, or `WB_KEEP_RUNNING=true`.

### Can my agent read files outside my recording folders through Wise Bucket?

No. Wise Bucket only reads inside the folders you declared, refuses symlinks that escape them, and has no tool that adds a folder. Your agent's own tools, a shell for example, are outside Wise Bucket's control; restrict them in the agent if you need to.

### Can I have several recording folders, or folders on a NAS?

Yes. Declare as many as you like with `wisebucket roots add <name> <path>`, network mounts included. Folders can't overlap.

### Can I use several agent windows at the same time?

Yes. Each window starts its own Wise Bucket process, and they share the same database.

### Where is my data, and how do I back it up?

`wisebucket config path` shows the locations. To back up, run `wisebucket backup wb.dump`; to restore, run `restore wb.dump --yes`. Your recordings are never part of a backup: they stay where they are.
