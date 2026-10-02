# User guide

Everything you need to install Wise Bucket, point it at your recordings, and connect it to your AI agent.

- [Install](#install)
- [Set up](#set-up)
- [Recording folders](#recording-folders)
- [Connect your agent](#connect-your-agent)
- [Everyday use](#everyday-use)
- [Where things live](#where-things-live)
- [Uninstall](#uninstall)

When something goes wrong, see [Troubleshooting and FAQ](troubleshooting.md). Every command and setting is listed in the [CLI](reference/cli.md) and [configuration](reference/configuration.md) references.

## Install

```sh
curl -LsSf https://github.com/MBagory/wise-bucket/releases/latest/download/wisebucket-installer.sh | sh
wisebucket --version
```

The installer puts `wisebucket` and its short alias `wbk` in `~/.cargo/bin` and adds that directory to your `PATH` (open a new terminal afterwards). To update, run it again. You also need an MCP-capable agent: Claude Code, OpenAI Codex, Cursor, Kilo Code, Cline…

| | Requirement |
| --- | --- |
| macOS | 15 (Sequoia) or later, Apple Silicon or Intel |
| Linux | x86_64 or arm64 with glibc 2.34 or later: Ubuntu 22.04+, Debian 12+, RHEL / Rocky / Alma 9+, Fedora, Amazon Linux 2023, Arch. Not Alpine. |
| Windows | through [WSL2](#windows-wsl2) |
| Disk | about 250 MB (PostgreSQL, pgvector, an empty database) |

`setup` downloads PostgreSQL and the pgvector extension, both prebuilt and checksum-verified, so no compiler is needed. On Linux, PostgreSQL uses a few system libraries that desktops and servers already have. A minimal container image may lack them; install them with:

```sh
sudo apt install libssl3 libxml2 libzstd1 liblz4-1 libgssapi-krb5-2 zlib1g tzdata   # Debian, Ubuntu
sudo dnf install openssl-libs libxml2 libzstd lz4-libs krb5-libs zlib tzdata         # RHEL family, Fedora
```

**Build from source** instead (needs [Rust](https://rustup.rs)): `cargo install --git https://github.com/MBagory/wise-bucket wisebucket --locked`.

### Windows: WSL2

Native Windows support is planned. Today, run Wise Bucket inside WSL2, where it behaves exactly as on Linux:

1. Install WSL2 with Ubuntu: `wsl --install -d Ubuntu`.
2. Inside Ubuntu, run the install command above.
3. Run your agent **from WSL** (for example, start `claude` in the WSL terminal, or use VS Code's WSL remote) so it can launch the Linux binary.
4. Recordings on the Windows side are reachable as `/mnt/c/...` and can be declared as recording folders. Access through `/mnt/c` is slower; for large recordings, prefer a folder inside the WSL file system.

## Set up

Run this once per machine:

```text
$ wisebucket setup
Wise Bucket setup
✔ state directory: ~/.local/share/wisebucket
… Downloading https://github.com/theseus-rs/postgresql-binaries/…/postgresql-17.11.0-x86_64-unknown-linux-gnu.tar.gz
… Downloading https://github.com/MBagory/wise-bucket/releases/…/pgvector-0.8.6-pg17.11.0-x86_64-unknown-linux-gnu.tar.gz
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

**What happens:**
1. **Download:** PostgreSQL 17.11 and pgvector 0.8.6 (prebuilt for that exact PostgreSQL) are downloaded and checked against pinned SHA-256 checksums.
2. **Database:** the database is created, with generated passwords stored in a file only you can read.
3. **Your answers** are written to your user configuration.

`setup` is idempotent: re-run it any time and it only does what's missing.

**Non-interactive** (scripts, CI):

```sh
wisebucket setup --yes --root bags=~/robot-logs/bags --root flights=/mnt/nas/px4
```

## Recording folders

A recording folder (a *data root*) is a named folder that Wise Bucket may read. It can hold recordings at any depth: rosbag2 folders (`metadata.yaml` plus `.mcap`/`.db3` files), loose `.mcap` files, flight logs… Nothing outside your folders is reachable.

```sh
wisebucket roots add bags ~/robot-logs/bags --robot rover-b
wisebucket roots add flights /Volumes/lab-nas/px4      # network mounts work
wisebucket roots list
wisebucket roots check      # readable? overlapping? how many recordings?
wisebucket roots remove flights                          # the folder itself is never touched
```

**Rules:**
- **Names** are short and stable (`bags`, `lab-nas`): lowercase letters, digits, `-` and `_`, at most 32 characters. Recordings are referenced as `name:relative/path`, so if a mount point moves, you only update that folder's path.
- **No overlap:** a folder inside another declared folder is refused.
- **No escapes:** symlinks must stay inside their folder.
- **Read-only:** Wise Bucket never modifies, moves or deletes recordings.
- **Your decision only:** Wise Bucket gives your agent no tool to add a folder. You add folders from the command line or a configuration file.
- **Restart the agent session** after changing folders: the server reads its configuration when it starts.

**No recordings yet?** `wisebucket demo` downloads public sample files (MCAP, ROS bags, ULog, DataFlash, MAVLink, CAN, MDF4, Parquet) and declares them as the `demo` folder. See [Try it with sample data](../README.md#try-it-with-sample-data) for the formats and the investigations they support.

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

`setup` only prints these entries; it never edits your agent's settings. Re-run `setup` to see them again (it skips everything already done).

**Check the connection.** Start a new agent session in any folder (in Claude Code, `/mcp` lists `wise-bucket`). Then ask *"Is Wise Bucket ready?"*. The agent calls `server_info` and reports:
- the version
- the database status
- the recording folders it may use

Optional, for Claude Code: to be asked before each Wise Bucket tool call, add this to `.claude/settings.json`:

```json
{ "permissions": { "ask": ["mcp__wise-bucket"] } }
```

## Everyday use

| You want to… | Run |
| --- | --- |
| See which agent windows are connected | `wisebucket db status` |
| Check that everything works | `wisebucket doctor` |
| See your settings and where each one comes from | `wisebucket config show --origin` |
| Back up your data | `wisebucket backup wb.dump` |
| Restore a backup (replaces current data) | `wisebucket restore wb.dump --yes` |
| Start or stop the managed database by hand | `wisebucket db start` · `db stop` |

- **Several windows at once:** each agent window starts its own Wise Bucket process, and they share one database. The managed PostgreSQL starts with the first session (about 2 s) and stops when the last one ends. To keep it running, set `keep_running = true` under `[database]` in your user configuration, or `WB_KEEP_RUNNING=true`.
- **Backups** contain Wise Bucket's own data only. Your recordings stay where they are and are never part of a backup.
- **Machine-readable output:** `config`, `roots`, `db status` and `doctor` accept `--json`.

## Where things live

| What | Where |
| --- | --- |
| Your settings and recording folders | Linux `~/.config/wisebucket/config.toml`, macOS `~/Library/Application Support/wisebucket/config.toml` |
| How your agent starts Wise Bucket | Your agent's user-level MCP settings (e.g. `claude mcp get wise-bucket`) |
| State directory | Linux `~/.local/share/wisebucket/`, macOS `~/Library/Application Support/wisebucket/`. To use another disk, set `state_dir = "…"` in your settings **before** running `setup` |

```text
<state directory>/
├── runtime/          PostgreSQL + pgvector binaries
├── pg/               the database
├── secrets/db.toml   generated passwords (readable only by you)
├── run/              Unix socket (private directory; nothing listens on the network)
└── logs/             server.log, postgres.log
```

**Settings precedence**, highest first: command-line flags > `WB_*` environment variables > user configuration > defaults. Annotated example: [user configuration](examples/user-config.toml).

## Uninstall

```sh
wisebucket db stop      # if it is running
rm ~/.cargo/bin/wisebucket ~/.cargo/bin/wbk
```

Your data stays in the state directory (`wisebucket config path` shows where). Delete it yourself if you no longer need it; Wise Bucket never deletes it.
