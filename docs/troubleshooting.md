# Troubleshooting and FAQ

**Start with `wisebucket doctor`.** It checks your configuration, the state directory, the store, SQL and your recording folders. Each problem comes with an error code and a link to its fix in the [error reference](reference/errors.md). Add `-v` to any command for diagnostic logs. The MCP server logs to `<state directory>/logs/server.log`.

- [Troubleshooting](#troubleshooting)
- [FAQ](#faq)

## Troubleshooting

### `wise-bucket` doesn't appear in my agent

- Register it once for your user with the command printed at the end of `wisebucket setup`, then start a **new** agent session. In Claude Code, `claude mcp get wise-bucket` shows the entry and `/mcp` lists it.
- Check that the entry's `command` path exists. After reinstalling or moving the binary, re-run `setup` and register the printed entry again (in Claude Code: `claude mcp remove --scope user wise-bucket` first).
- Run the entry's command by hand. It should wait silently for input (press Ctrl-C to quit). If it prints an error instead, that error tells you what is wrong.

### The build fails with `Could not find protoc` or a C compiler error

Building from source needs a C compiler and `protoc`, the Protocol Buffers compiler: see [Install](guide.md#install). If `protoc` is installed somewhere unusual, point the build at it with `PROTOC=/path/to/protoc`.

### A recording stays `pending`

Recordings are extracted only while an agent session is open: open one (or ask the agent *"Is Wise Bucket ready?"*). A file changed in the last few seconds waits until it stops changing. If it stays `pending` for minutes with a session open, check that extraction is not `paused` (below) and look at `<state directory>/logs/server.log`.

### A recording is `failed` (`recording_unreadable`)

```sh
wisebucket sql "SELECT recording, error FROM recordings WHERE status = 'failed'"
```

The message says why: a truncated or damaged file, or a feature Wise Bucket cannot read yet. The recording itself is untouched, and it is retried automatically when the file changes. If the file opens in its own tools, please open an issue with the message.

### Extraction is `paused`

The store's disk has less than 10 GB free. Free some space (deleting the store is safe: see [Reset](guide.md#reset)), or move the store to another disk with `state_dir` in your configuration. Extraction resumes on its own.

### A stream has no data columns, or `decoded` is false

`SELECT stream, decoded, notes FROM streams` says why:
- **Images, point clouds, maps, video:** only their timing and message sizes are kept, on purpose.
- **A custom ROS 2 type in a Humble `.db3` bag:** the bag doesn't store the definition, and only standard types are bundled. Recording with MCAP storage (`ros2 bag record -s mcap`) stores the definitions.
- **ROS 1, protobuf or JSON payloads in MCAP:** not decoded yet; timing and sizes are kept.

### The times of a CSV or Parquet file are wrong

`t` comes from a guessed column, and the stream's `notes` say which one. The unit comes from a suffix (`time_ms`, `t_us`…) and defaults to seconds. Rename the column in the file, or compute the right time in your query from the original column, which is always kept.

### My query fails (`query_failed`)

The message comes from the query engine and names the problem: an unknown table or column, a syntax error, a write, or a query running past 30 seconds. Remember to quote dotted and upper-case column names (`"linear_acceleration.z"`). The [tables reference](reference/tables.md) lists every column, with recipes.

### A recording folder is reported invalid or overlapping

- `root_invalid`: the folder doesn't exist, isn't readable, or its name isn't valid. `wisebucket roots check` shows the details.
- `root_overlap`: one folder contains another. Keep the outer one, or split them.

### `demo` fails with `download_failed` or `checksum_mismatch`

- `download_failed`: check network access to `github.com` (proxy, firewall), then re-run `demo`. Completed downloads are kept.
- `checksum_mismatch`: the downloaded file is corrupted or was tampered with. Re-run `demo`: files with a wrong checksum are downloaded again. If it happens again, please report it (see [SECURITY.md](../SECURITY.md)).

### `config_invalid` after updating

The configuration format changed: a data root's `robot = "…"` is now `hardware = "…"`. Edit the line in your user configuration (`wisebucket config path` shows where).

### WSL2

- Run the agent **inside WSL** so it can start the Linux binary.
- Folders under `/mnt/c/...` work but are slow to scan; for large recordings, prefer the WSL file system.

### Still stuck?

Open an issue with the output of `wisebucket doctor --json`; it contains no secrets.

## FAQ

### Does Wise Bucket call an LLM or use my API key?

No. Wise Bucket never calls a model. Your agent owns the model, your key and your token budget.

### Do my recordings leave my machine?

No. Wise Bucket reads them where they are and keeps its decoded copy on your machine. Only the tool results your agent sends to its model leave it, under your agent's terms; see [SECURITY.md](../SECURITY.md#privacy).

### Why copy my recordings instead of reading them on each question?

Reading a bag means decoding it, which takes seconds to minutes; querying the copy takes milliseconds to seconds, and works across all your recordings at once. Wise Bucket trades disk space for that speed. The copy leaves out images and point clouds, and you can delete it any time. The reasons and the alternatives: [ADR 0012](decisions/0012-trade-storage-for-query-speed.md).

### Is there a database to install or keep running?

No. The store is a folder of [Lance](https://github.com/lance-format/lance) files, and queries run inside Wise Bucket ([Apache DataFusion](https://datafusion.apache.org)). Nothing runs in the background when no agent session is open. ([ADR 0011](decisions/0011-no-database.md))

### Does Wise Bucket compute metrics for me?

Not in advance. Your agent writes a SQL query for each question (rates, gaps, windows, percentiles, joins between streams) and shows you the query behind each number. You can run the same queries with `wisebucket sql`. ([ADR 0010](decisions/0010-metrics-are-sql-written-by-the-agent.md))

### Can my agent read files outside my recording folders through Wise Bucket?

No. Wise Bucket only reads inside the folders you declared, refuses symlinks that escape them, and has no tool that adds a folder. Its SQL can't read or write files: it sees only the store. Your agent's own tools, a shell for example, are outside Wise Bucket's control; restrict them in the agent if you need to.

### Can I have several recording folders, or folders on a NAS?

Yes. Declare as many as you like with `wisebucket roots add <name> <path>`, network mounts included. Folders can't overlap.

### Can I use several agent windows at the same time?

Yes. Each window starts its own Wise Bucket process; they share the store and the extraction work, and never extract the same file twice.

### Where is my data, and how do I back it up?

`wisebucket config path` shows the locations. There is nothing to back up besides your recordings and your configuration file: the store is rebuilt from your recordings. Note that recordings deleted from your folders keep their data in the store (status `missing`) until you delete the store.
