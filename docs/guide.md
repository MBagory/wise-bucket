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

| | Linux (x86_64, arm64) | macOS (Apple Silicon, Intel) | Windows |
| --- | --- | --- | --- |
| Wise Bucket | ✅ | ✅ | through [WSL2](#windows-wsl2) |
| C toolchain (one-time pgvector build) | `sudo apt install build-essential` (Debian/Ubuntu) or `sudo dnf install gcc make` (Fedora) | `xcode-select --install` | inside WSL2 |
| Disk | about 250 MB (PostgreSQL, pgvector, an empty database) | same | same |

You also need [Rust](https://rustup.rs) (until pre-built binaries are published) and an MCP-capable agent: Claude Code, OpenAI Codex, Cursor, Kilo Code, Cline…

```sh
git clone https://github.com/MBagory/wise-bucket
cd wise-bucket
cargo install --path crates/wb-server --locked
wisebucket --version
```

`cargo install` puts `wisebucket` and its short alias `wbk` in `~/.cargo/bin`. Make sure that directory is on your `PATH`.

### Windows: WSL2

Native Windows support is planned. Today, run Wise Bucket inside WSL2, where it behaves exactly as on Linux:

1. Install WSL2 with Ubuntu: `wsl --install -d Ubuntu`.
2. Inside Ubuntu, install Rust and `build-essential`, then follow the steps above.
3. Run your agent **from WSL** (for example, start `claude` in the WSL terminal, or use VS Code's WSL remote) so it can launch the Linux binary.
4. Recordings on the Windows side are reachable as `/mnt/c/...` and can be declared as recording folders. Access through `/mnt/c` is slower; for large recordings, prefer a folder inside the WSL file system.

## Set up

Run this once per machine:

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
```

**What happens:**
1. **Download:** PostgreSQL 17.11 is downloaded and checked against a pinned SHA-256 checksum.
2. **Build:** pgvector 0.8.6 is compiled against it, which takes about 15 s.
3. **Database:** the database is created, with generated passwords stored in a file only you can read.
4. **Your answers** are written to your user configuration.

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

**Folders inside a repository** can be shared with your team through the project configuration. Their paths must be relative and stay inside the repository:

```sh
cd ~/code/rover
wisebucket roots add repo-bags ./bags --in-project
```

## Connect your agent

Run this once per repository:

```sh
cd ~/code/rover
wisebucket init --robot rover-b
```

```text
✔ wrote ~/code/rover/.wisebucket/config.toml
✔ wrote ~/code/rover/.mcp.json
```

- **`.wisebucket/config.toml`** holds the project name and default robot. Commit it.
- **`.mcp.json`** tells the agent how to start Wise Bucket: the absolute binary path and `serve --project <repo>`. It contains no data paths and no secrets. Whether to commit it is your call, since the binary path is specific to your machine.

**Per agent:**

| Agent | How to connect |
| --- | --- |
| **Claude Code** | `wisebucket init` writes `.mcp.json` in the repository |
| **Kilo Code** | `wisebucket init --harness kilo` writes `.kilocode/mcp.json` |
| **Cursor** | Copy the entry printed by `wisebucket init --print` into `.cursor/mcp.json` (same format) |
| **OpenAI Codex** | Add the entry to `~/.codex/config.toml` (below) |
| **Cline** | `wisebucket init --harness cline` prints the entry for Cline's MCP settings |

```toml
# ~/.codex/config.toml
[mcp_servers.wise-bucket]
command = "/home/me/.cargo/bin/wisebucket"
args = ["serve", "--project", "/home/me/code/rover"]
```

`init` never overwrites your files. If a configuration already lists other servers, `init` prints the entry to add, and `--merge` adds it for you while keeping the others. `--print` only shows what would be written.

**Check the connection.** Open your agent in the repository and approve the `wise-bucket` server (in Claude Code, `/mcp` lists it). Then ask *"Is Wise Bucket ready?"*. The agent calls `server_info` and reports:
- the version
- the database status
- the project and default robot
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
| The project's name and default robot | `<repo>/.wisebucket/config.toml` |
| How your agent starts Wise Bucket | `<repo>/.mcp.json` (or your agent's equivalent) |
| State directory | Linux `~/.local/share/wisebucket/`, macOS `~/Library/Application Support/wisebucket/`. To use another disk, set `state_dir = "…"` in your settings **before** running `setup` |

```text
<state directory>/
├── runtime/          PostgreSQL + pgvector binaries
├── pg/               the database
├── secrets/db.toml   generated passwords (readable only by you)
├── run/              Unix socket (private directory; nothing listens on the network)
└── logs/             server.log, postgres.log
```

**Settings precedence**, highest first: command-line flags > `WB_*` environment variables > project configuration > user configuration > defaults. Annotated examples: [user configuration](examples/user-config.toml), [project configuration](examples/project-config.toml).

## Uninstall

```sh
wisebucket db stop      # if it is running
cargo uninstall wb-server
```

Your data stays in the state directory (`wisebucket config path` shows where). Delete it yourself if you no longer need it; Wise Bucket never deletes it.
