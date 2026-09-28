# Connect your harness

Wise Bucket runs as an MCP server that your AI harness starts on demand. `init` configures a robot repository for it.

> This page has an executable twin, the `docs_connect_your_harness` test, which runs these exact steps in CI.

## 1. Initialize your robot repository

```sh
cd ~/code/rover
wise-bucket-server init --robot rover-b
```

```text
✔ wrote ~/code/rover/.wisebucket/config.toml
✔ wrote ~/code/rover/.mcp.json
Next: open Claude Code in ~/code/rover (approve the project MCP server when asked), check that `/mcp` lists `wise-bucket`, then ask: "call server_info".
```

- `.wisebucket/config.toml` holds the project name and default robot. **Commit it.**
- `.mcp.json` tells Claude Code how to start Wise Bucket: the absolute path of the binary and `serve --project <repo>`. It contains no data paths or secrets. Whether to commit it is up to you: the binary path is machine specific.

`init` **never overwrites** existing files. If `.mcp.json` already lists other servers, `init` prints the entry to add; `init --merge` adds it for you and keeps the other servers. `init --print` only shows what would be written.

## 2. Claude Code

1. Open Claude Code in the repository: `claude`.
2. Approve the project MCP server `wise-bucket` when asked.
3. Type `/mcp`: `wise-bucket` should be listed as connected.
4. Ask: **"call server_info"**. Expected answer:

   ```text
   Wise Bucket 0.1.0 (milestone M0)
   database: managed · PostgreSQL 17.11 · pgvector 0.8.6 · 1 active session(s)
   project: rover (default robot: rover-b)
   data roots: bags → /home/me/robot-logs/bags
   ```

Optional: to always be asked before Wise Bucket tools run, add a permission rule in `.claude/settings.json`:

```json
{ "permissions": { "ask": ["mcp__wise-bucket"] } }
```

## Kilo Code

```sh
wise-bucket-server init --harness kilo
```

This writes `.kilocode/mcp.json`. Enable the `wise-bucket` server in Kilo Code's MCP settings.

## Cline

Cline reads MCP servers from its global settings. Print the entry and paste it in *Cline › MCP Servers › Configure*:

```sh
wise-bucket-server init --harness cline
```

## Several windows at once

You can open several harness sessions on the same machine. They share one database. The managed PostgreSQL starts with the first session and stops when the last one ends; `wise-bucket-server db status` lists the active sessions.

## Checking everything

```sh
wise-bucket-server doctor
```

```text
✔ configuration      loaded (~/.config/wisebucket/config.toml)
✔ state directory    ~/.local/share/wisebucket · 91.4 GB free
✔ postgresql         17.11.0 installed
✔ pgvector           0.8.6 installed
✔ socket             ~/.local/share/wisebucket/run
✔ database           managed · PostgreSQL 17.11 · vector 0.8.6 · pg_trgm 1.6 · 0 active session(s)
✔ data root          bags → /home/me/robot-logs/bags
✔ project            rover (/home/me/code/rover)
✔ harness config     /home/me/code/rover/.mcp.json → `wise-bucket`
```

Each problem comes with an error code and a link to its fix in the [error reference](../reference/errors.md).
