# How it works

```text
 Engineer ⇄ AI harness (Claude Code · Cline · Kilo)
                 │  MCP over stdio
                 ▼
        wise-bucket-server (Rust)
          │                    │ read-only
          ▼                    ▼
 PostgreSQL + pgvector    Data roots
 (local, managed)         (your recording folders)
```

- **Your harness** (e.g. Claude Code) starts `wise-bucket-server serve` as an MCP server when you open your robot repository. The command is written in `.mcp.json` by `wise-bucket-server init`.
- **The server** is a single Rust binary. It exposes MCP tools; in M0 there is one, `server_info`.
- **The database** is PostgreSQL with the pgvector extension. By default Wise Bucket downloads, runs and stops a private instance (the *managed* mode); you can also point it at your own server.
- **Data roots** are the folders you declare. Wise Bucket only reads inside them and never modifies your recordings.

## Lifecycle

- The first harness session starts the managed PostgreSQL (about 2 s); later sessions reuse it.
- When the last session ends, PostgreSQL stops, unless `keep_running` is set.
- Each session is recorded (client name and version, start and end) so later milestones can say *which conversation* stated a fact.

## Where things are stored

| What | Where |
| --- | --- |
| User configuration | Linux `~/.config/wisebucket/config.toml`, macOS `~/Library/Application Support/wisebucket/config.toml` |
| Project configuration | `<robot-repo>/.wisebucket/config.toml` (commit it) |
| Harness configuration | `<robot-repo>/.mcp.json` |
| Database, logs, caches | the *state directory*: Linux `~/.local/share/wisebucket`, macOS `~/Library/Application Support/wisebucket` |
| PostgreSQL binaries | `<state directory>/runtime` |
| Your recordings | where they already are, inside your data roots |

Run `wise-bucket-server config show --origin` to see the effective values and where each comes from.
