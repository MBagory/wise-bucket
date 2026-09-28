# Configure: database and recording folders

Wise Bucket is configured in three places, each with its own owner:

| File | Who writes it | What it holds |
| --- | --- | --- |
| **User config**: Linux `~/.config/wisebucket/config.toml`, macOS `~/Library/Application Support/wisebucket/config.toml` | you, once per machine (`setup`, `roots add`) | state directory, database mode, **your recording folders** |
| **Project config**: `<robot-repo>/.wisebucket/config.toml` | `init`, committed with your robot code | project name, default robot, folders *inside* the repository |
| **Harness config**: `<robot-repo>/.mcp.json` | `init` | how your harness starts Wise Bucket; no data paths |

Precedence, highest first: command-line flags > `WB_*` environment variables > project config > user config > defaults. `wise-bucket-server config show --origin` shows every effective value and where it comes from. The [configuration reference](../reference/configuration.md) lists every key.

## Run setup

```text
$ wise-bucket-server setup
Wise Bucket setup
  user configuration: ~/.config/wisebucket/config.toml
? State directory (database, caches, logs) [~/.local/share/wisebucket]:
✔ state directory: ~/.local/share/wisebucket
? Database:
> Managed local PostgreSQL + pgvector (recommended)
  My own PostgreSQL server (>= 15 with pgvector >= 0.8)
… Downloading PostgreSQL 17 …
… Building pgvector 0.8.6 (about 15 s)
✔ PostgreSQL 17.11.0 + pgvector 0.8.6 ready
✔ database ready: PostgreSQL 17.11, vector 0.8.6, pg_trgm 1.6
? Add a folder containing recordings (rosbags, MCAP, …)? yes
  Folder path: ~/robot-logs/bags
  Short name [bags]:
  Default robot for these recordings (optional): rover-b
✔ root "bags" → /home/me/robot-logs/bags · 37 candidate recordings (31 MCAP, 6 rosbag2 folders, 0 ROS 1, 0 ULog)
? Add another folder? no
✔ configuration written to ~/.config/wisebucket/config.toml
```

`setup` is idempotent: run it again at any time; it only does what is missing.

**Non-interactive (scripts, CI):**

```sh
wise-bucket-server setup --yes --root bags=~/robot-logs/bags --root flights=/mnt/nas/px4
```

## Data roots: the folders Wise Bucket may read

A **data root** is a named folder containing recordings, at any depth: rosbag2 folders (`metadata.yaml` plus `.mcap`/`.db3` files), loose `.mcap` files, and later flight logs or CAN captures. Wise Bucket **only** reads inside roots and never modifies what it reads.

```sh
wise-bucket-server roots add bags ~/robot-logs/bags --robot rover-b
wise-bucket-server roots add nas-flights /Volumes/lab-nas/px4
wise-bucket-server roots list
wise-bucket-server roots check      # readable? overlapping? how many recordings?
wise-bucket-server roots remove nas-flights   # the folder itself is never touched
```

Rules:

- **Names** are short and stable (`bags`, `nas-flights`): lowercase letters, digits, `-`, `_`. Later, recordings are referenced as `name:relative/path` (for example `bags:2026-09-24/dock_run3`). If a mount point moves, update the root's `path` and every reference still works.
- **No overlap:** a root cannot contain another root.
- **Symlinks** must stay inside their root.
- **Network drives** are fine (slower to scan).
- **Restart your harness session** after changing roots: the server reads its configuration when it starts.

There is deliberately **no MCP tool to add a root**: the AI cannot widen its own access.

### Folders inside the robot repository

Recordings committed or stored inside the repository can be declared in the project config, shared with your team. Paths must be relative and stay inside the repository:

```sh
cd ~/code/rover
wise-bucket-server roots add repo-bags ./bags --in-project
```

## Bring your own PostgreSQL

Instead of the managed database, you can use a PostgreSQL **15 or newer** server with **pgvector 0.8 or newer** and the `pg_trgm` contrib extension, for example a shared lab server or the `pgvector/pgvector:pg17` Docker image.

1. As a superuser, create a database and a user that owns it, then the extensions:

   ```sql
   CREATE ROLE wisebucket LOGIN PASSWORD '…';
   CREATE DATABASE wisebucket OWNER wisebucket;
   \c wisebucket
   CREATE EXTENSION vector;
   CREATE EXTENSION pg_trgm;
   ```

2. Give Wise Bucket the URL through the environment (it is never written to a file):

   ```sh
   export WB_DATABASE_URL='postgres://wisebucket:…@db.lab.local:5432/wisebucket'
   wise-bucket-server setup --yes
   ```

`init` then writes `.mcp.json` so that your harness passes `WB_DATABASE_URL` from its environment.

## Examples

User configuration:

```toml
{{#include ../fragments/user-config.example.toml}}
```

Project configuration:

```toml
{{#include ../fragments/project-config.example.toml}}
```

Next: [Connect your harness](connect-your-harness.md).
