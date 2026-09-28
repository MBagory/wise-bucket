# Trust and privacy

## What stays on your machine

- **Your recordings.** Wise Bucket reads them in place, inside your data roots, and never uploads, copies or modifies them.
- **The database** (knowledge, investigations, metrics, sessions). It runs locally; in managed mode it listens **only on a Unix socket** in a private directory, never on the network.
- **Credentials.** The managed database password is generated at setup and stored in a file readable only by you (`<state directory>/secrets/db.toml`, mode 0600). An external database URL is read from the environment or the command line and is never written to a configuration file.

## What reaches your LLM provider

Your harness sends **tool results** to its model. Wise Bucket's tool results are bounded summaries (in M0: versions, the project name, root names and paths). Future milestones will return metrics and short excerpts, never whole recordings. Everything the harness sends to its provider follows *its* privacy terms, not Wise Bucket's.

## Boundaries Wise Bucket enforces

- **Data roots.** Only declared folders can be read. Symlinks that escape a root are refused. Roots can only be added by you through the command line or a configuration file; no MCP tool can add one.
- **No model calls.** Wise Bucket never contacts an LLM.
- **No network access at runtime.** Only `setup` downloads files (PostgreSQL, pgvector sources), each verified against a pinned SHA-256 checksum.

## What Wise Bucket cannot enforce

Your harness usually has its own tools, for example a shell. Those can read any file you can read, regardless of Wise Bucket's data roots. If you need the AI to stay inside your recording folders, also restrict the harness (for example, Claude Code's permission rules and sandbox).

To report a security problem, see [SECURITY.md](https://github.com/MBagory/wise-bucket/blob/main/SECURITY.md).
