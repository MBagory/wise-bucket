<div align="center">

<img src="docs/assets/banner.svg" alt="Wise Bucket: long-term context and insights for your time-series data" width="100%">

**An open-source MCP server that gives R&D teams long-term context and insights on their time-series data.**

[![CI](https://github.com/MBagory/wise-bucket/actions/workflows/ci.yml/badge.svg)](https://github.com/MBagory/wise-bucket/actions/workflows/ci.yml)
[![Docs](https://img.shields.io/badge/docs-guide-blue)](docs/guide.md)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rust-1.94%2B-orange.svg)](Cargo.toml)
![Status: alpha](https://img.shields.io/badge/status-alpha-red)

[Overview](#overview) · [Getting started](#getting-started) · [Sample data](#try-it-with-sample-data) · [Everyday use](#everyday-use) · [Platforms](#platforms) · [Docs](#documentation) · [Contributing](#contributing)

</div>

## Overview

Wise Bucket connects your **AI agent** to the **time-series data on your machine**, in the formats R&D teams actually record:

- Tabular exports: CSV, Parquet
- Test benches and instruments: ASAM MDF4 (`.mf4`), NI TDMS (`.tdms`), HDF5 (`.h5`)
- Buses and telemetry: CAN captures (candump, Vector ASC/BLF) decoded with a DBC file, MAVLink (`.tlog`)
- Robotics and embedded logs: MCAP / ROS bags, PX4 ULog (`.ulg`), ArduPilot DataFlash (`.bin`)

The goal is to give the agent **the full context it usually lacks**, by *standardizing metadata* and *asking you for whatever seems missing*.

*Next*, it will compute and store **signal-processing metrics** and **embeddings**, to surface *analogies* and *weak signals* across all your recordings. Whether you ask a new question or come back to an old one, your agent will answer with **long-term context**.

> [!WARNING]
> **Work in progress.** Wise Bucket is in early alpha and under active development. Commands, configuration and on-disk formats may change without notice, and it is not ready for production use. Feedback and [issues](https://github.com/MBagory/wise-bucket/issues) are welcome.

## Getting started

### 1. Install

**Requirements:** Linux or macOS (Windows through WSL2), [Rust](https://rustup.rs), and a C compiler (`xcode-select --install` on macOS, `build-essential` on Debian/Ubuntu).

```sh
git clone https://github.com/MBagory/wise-bucket && cd wise-bucket
cargo install --path crates/wb-server --locked
```

This installs `wisebucket` and its short alias `wbk`.

### 2. Set up your machine (once, about a minute)

```sh
wisebucket setup
```

`setup` only asks which folders hold your recordings. Re-run it any time: **it only does what's missing**.

<details>
<summary>Example session</summary>

```text
$ wisebucket setup
Wise Bucket setup
✔ state directory: ~/.local/share/wisebucket
… Downloading https://github.com/theseus-rs/postgresql-binaries/…/postgresql-17.11.0-x86_64-unknown-linux-gnu.tar.gz
… Building pgvector 0.8.6 (about 15 s)
✔ PostgreSQL 17.11.0 + pgvector 0.8.6 ready
✔ database ready: PostgreSQL 17.11, vector 0.8.6, pg_trgm 1.6
? Add a folder containing recordings (rosbags, MCAP, …)? › yes
  Folder path › ~/robot-logs/bags
  Short name › bags
  Default robot for these recordings (optional) › rover-b
✔ root "bags" → /home/me/robot-logs/bags · 37 candidate recordings (31 MCAP, 6 rosbag2 folders, 0 ROS 1, 0 ULog)
? Add another folder? › no
✔ configuration written to ~/.config/wisebucket/config.toml

Connect your agent (once, for all your folders). Claude Code:
claude mcp add --scope user wise-bucket -- /home/me/.cargo/bin/wisebucket serve
Other agents (Kilo Code, Cline, Cursor, …): add this to their global MCP settings:
{ "mcpServers": { "wise-bucket": { "command": "/home/me/.cargo/bin/wisebucket", "args": ["serve"] } } }
```

</details>

> [!TIP]
> **For scripts and CI:** `wisebucket setup --yes --root bags=~/robot-logs/bags`

### 3. Connect your agent (once)

Run the command `setup` printed at the end. Wise Bucket is registered for your user, so it works in every folder: there is nothing to do per repository.

| Agent | How to connect |
| --- | --- |
| **Claude Code** | `claude mcp add --scope user wise-bucket -- /home/me/.cargo/bin/wisebucket serve` |
| **Kilo Code, Cline, Cursor** | Add the JSON entry printed by `setup` to the agent's global MCP settings |
| **OpenAI Codex** | Add the entry to `~/.codex/config.toml` (below) |

```toml
# ~/.codex/config.toml
[mcp_servers.wise-bucket]
command = "/home/me/.cargo/bin/wisebucket"
args = ["serve"]
```

> [!NOTE]
> `setup` **only prints** these entries: it never edits your agent's settings. Re-run `setup` any time to see them again.

### 4. Talk to your agent

Start a new agent session, in any folder (in Claude Code, `/mcp` lists `wise-bucket`). Then just ask:

```text
you   › Is Wise Bucket ready? Which recording folders can you use?

agent › ⏺ wise-bucket · server_info
        Wise Bucket 0.1.0 is connected and its database is running
        (PostgreSQL 17.11, pgvector 0.8.6, 1 active session).
        I can use one recording folder: bags → /home/me/robot-logs/bags.
```

<details>
<summary>When something isn't right, the answer tells you what to do</summary>

```text
you   › Is Wise Bucket ready?

agent › ⏺ wise-bucket · server_info
        Not yet: the database hasn't been set up on this machine
        (db_not_initialized). Run `wisebucket setup` once,
        then start a new session.
        Docs: https://github.com/MBagory/wise-bucket/blob/main/docs/reference/errors.md#db_not_initialized
```

</details>

## Try it with sample data

No recordings at hand? `demo` downloads small **public sample files**, one set per format, and declares them as a data root named `demo`:

```sh
wisebucket demo                 # pick formats from a list
wisebucket demo ros2-mcap ulog  # or name them
wisebucket demo --all           # everything (about 4 MB)
wisebucket demo --list          # formats, sizes and licenses
```

The files go to `<state dir>/demo/<format>/`. They are fetched from their upstream projects at a pinned commit and checked against a SHA-256, and they **stay under their own licenses** (Apache-2.0, MIT, BSD-3-Clause, LGPL-3.0): this repository does not include them. Re-running `demo` only downloads what is missing. Start a new agent session to see the `demo` root; remove it with `wisebucket roots remove demo` and by deleting the folder.

### Investigations to try

Wise Bucket currently **lists and counts** recordings; it does not read them yet. Each line below says which milestone makes the investigation runnable.

| Format | Sample | Ask your agent | Runnable from |
| --- | --- | --- | --- |
| *all* | everything you downloaded | *"Which recordings does Wise Bucket see in the demo root?"* | now (M0) |
| `ros2-mcap` · `ros2-db3` | ROS 2 talker demo, 4.5 s: `/topic` (`std_msgs/String`) and `/rosout`, 10 messages each, as MCAP and as SQLite | *"Is `/topic` published at a steady rate?"* · *"Do the MCAP and SQLite copies hold the same messages?"* | M3 (MCAP), E2 (`.db3`) |
| `mcap` | ROS 1-encoded demo (`/chatter`, `/diagnostics`) and an MCAP spec conformance file | *"Which topics are in `demo.mcap`, and at what rates?"* · *"Does the conformance file decode to exactly ten messages?"* | M3 (conformance), E3 (ROS 1 encoding) |
| `ros1-bag` | demo bag plus one bag (`/chatter`, `/numbers`) stored uncompressed, LZ4 and BZ2 | *"Do the three compressions decode to the same `/numbers` sequence? Any gaps?"* | E3 (ROS 1) |
| `ulog` | PX4 autopilot on the bench, disarmed, 69 s, no barometer detected | *"Why is the barometric altitude flat?"* · *"What is the IMU noise floor at rest?"* · *"When does CPU load peak (83 %) and what runs then?"* | E3 (ULog), M4 (metrics) |
| `dataflash` | ArduPlane 3.8 software-in-the-loop log: attitude, PIDs, EKF, GPS, battery, vibration | *"How large is the roll tracking error (`ATT.DesRoll` vs `ATT.Roll`)?"* · *"Does battery voltage sag with current?"* · *"Are vibration levels within ArduPilot's limits?"* | E3 (DataFlash), M4 |
| `tlog` | one MAVLink 2 `BATTERY_STATUS` (12.59 V, 100 %) followed by a truncated packet | *"What battery state was reported?"* · *"Is the truncated tail reported instead of failing the whole file?"* | E3 (MAVLink) |
| `can` | Vector ASC capture (20 frames, 9 IDs, 4 error frames, 28 s), a BLF file, and an unrelated DBC | *"Which IDs are on the bus, how often, and when do the error frames occur?"* · *"Which frames can't this DBC decode?"* (all of them, so Wise Bucket should ask you for the right one) | E3 (CAN) |
| `mdf4` | MDF 4.10 file whose four channels have meaningless names | *"What do these channels measure?"* Wise Bucket cannot guess, so it should ask you to bind them | E3 (MDF4) |
| `parquet` | 8-row table covering the Parquet primitive types | no investigation: it checks the format of Wise Bucket's decoded cache | M3 |

## Everyday use

### Commands

| You want to… | Run |
| --- | --- |
| Add a recording folder (a NAS mount works too) | `wisebucket roots add flights /Volumes/lab-nas/px4` |
| See what your folders contain | `wisebucket roots check` |
| See which agent windows are connected | `wisebucket db status` |
| Check that everything works | `wisebucket doctor` |
| Back up (and restore) your data | `wisebucket backup wb.dump` · `restore wb.dump --yes` |
| See your settings and where each comes from | `wisebucket config show --origin` |
| Tab-complete commands in your shell | `wisebucket completions zsh > ~/.zfunc/_wisebucket` (also `bash`, `fish`) |


> [!IMPORTANT]
> After changing folders, **start a new agent session** to pick them up. Your recordings are **never modified, moved or included in backups**; `roots remove` only *forgets* a folder.

### Check your installation

`doctor` **checks everything** and links each problem to *its fix*:

<details>
<summary>Example output</summary>

```text
$ wisebucket doctor
✔ configuration      loaded (~/.config/wisebucket/config.toml)
✔ state directory    ~/.local/share/wisebucket · 84.6 GB free
✔ postgresql         17.11.0 installed
✔ pgvector           0.8.6 installed
✔ database           managed · PostgreSQL 17.11 · vector 0.8.6 · pg_trgm 1.6 · 0 active session(s)
✔ data root          bags → /home/me/robot-logs/bags
✖ data root          root "old-bags": /home/me/old-bags is not accessible
                     ↳ root_invalid · https://github.com/MBagory/wise-bucket/blob/main/docs/reference/errors.md#root_invalid

7 check(s), 1 failed, 0 warning(s)
```

</details>

### Where your files are

| What | Where |
| --- | --- |
| Your settings and recording folders | Linux `~/.config/wisebucket/config.toml`, macOS `~/Library/Application Support/wisebucket/config.toml` |
| How your agent starts Wise Bucket | Your agent's user-level MCP settings (e.g. `claude mcp get wise-bucket`) |
| Database, logs, generated passwords | Linux `~/.local/share/wisebucket/`, macOS `~/Library/Application Support/wisebucket/` (about 250 MB) |

## Platforms

| Platform | Status |
| --- | --- |
| Linux x86_64 / arm64 | ✅ |
| macOS Apple Silicon / Intel | ✅ |
| Windows | through WSL2 |

## Documentation

- [User guide](docs/guide.md): install, setup, recording folders, connecting your agent, everyday use
- [Troubleshooting and FAQ](docs/troubleshooting.md)
- Reference: [CLI](docs/reference/cli.md) · [Configuration](docs/reference/configuration.md) · [Errors](docs/reference/errors.md)
- [Security and privacy](SECURITY.md)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) and the [Code of Conduct](CODE_OF_CONDUCT.md).

```sh
cargo build
cargo test --workspace               # first run downloads PostgreSQL and builds pgvector (~1 min)
cargo run -p wb-server -- docs gen   # regenerate reference pages after changing the CLI, errors or config
cargo test -p wb-server --test samples -- --ignored  # per-format sample checks (downloads ~4 MB once)
```

The acceptance tests drive **the real binary** through **a real MCP client**. They cover two concurrent sessions, stop-after-last, recording folders, `doctor` and backup/restore.

## Security

**Recordings and the database stay on your machine.** Downloads are *pinned and verified*. What your agent sends to its own model follows your agent's terms. Report vulnerabilities **privately**: see [SECURITY.md](SECURITY.md).

## License

[Apache License 2.0](LICENSE). Built on [PostgreSQL](https://www.postgresql.org), [pgvector](https://github.com/pgvector/pgvector), [theseus-rs PostgreSQL binaries](https://github.com/theseus-rs/postgresql-binaries), the [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk) and [SQLx](https://github.com/launchbadge/sqlx). To cite Wise Bucket, use [CITATION.cff](CITATION.cff).
