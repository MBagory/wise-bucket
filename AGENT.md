# AGENT.md: context for coding agents

Concise context for AI coding agents working on this repository. Human-facing documentation lives in `docs/` and the README.

## What this project is

Wise Bucket: a Rust, local, stdio **MCP server** helping one robotics engineer run evidence-backed, iterative investigations (question → hypotheses → physical test → recording → analysis). The **harness** (Claude Code, Cline, Kilo…) owns the LLM, API keys, budgets and the conversation loop. **Wise Bucket never calls an LLM.**

## Decisions (do not revisit without the maintainer)

```yaml
language: rust (edition 2024, MSRV 1.94)
crates: [wb-core, wb-server]          # wb-data arrives in M3, wb-exec later; no empty crates ahead of time
store: postgresql 17 + pgvector >= 0.8 (managed via pinned theseus-rs binaries + pgvector built from source; or BYO URL)
embeddings_search: key feature from M1 (fastembed multilingual-e5-small, hybrid semantic + FTS + trigram)
readers: native rust (ROS 2 MCAP first); no Python readers; no Bagel
generated_code_execution: deferred (decision pending)
config: user TOML + project .wisebucket/config.toml + harness .mcp.json; precedence flags > WB_* env > project > user > defaults
data_roots: engineer-declared; no MCP tool may add or widen roots
platforms: linux + macos (windows via WSL2; native later)
license: Apache-2.0
delivery: one milestone at a time; stop after each for the maintainer to test
```

## Invariants

- stdout of `serve` carries MCP messages only; logs go to stderr and `<state>/logs/`.
- Every user-facing error is a `WbError` with a stable `code` and a documented fix (`crates/wb-core/src/error.rs`); `docs gen` renders `docs/src/reference/errors.md`.
- Secrets are never written to config files or printed (`mask_url`, 0600 secrets file).
- Downloads happen only in `setup`, always SHA-256 verified against pinned values.
- Recordings are never modified, moved or copied.
- The managed database listens only on a Unix socket (no TCP).
- Semantic database rows are append-only with `supersedes` (from M1); every row has `rev`, `session_id`, `created_at`.
- Documentation is part of every change (book pages, FAQ/troubleshooting, CHANGELOG).

## Layout

```
crates/wb-core     config, roots, errors, fsutil, paths, db (managed lifecycle, migrations, backup), session
crates/wb-server   CLI (clap) + MCP server (rmcp 3.x): src/cli.rs, src/commands/*, src/mcp.rs
crates/wb-core/migrations   plain-SQL sqlx migrations (schema `wb`, read-only views in `wb_views`)
crates/wb-server/tests      milestone acceptance tests (real binary + real MCP client)
docs/              mdBook (philosophy, get-started, reference [generated], faq, troubleshooting)
doc/demos          "try it yourself" guide per milestone; doc/spikes: spike outcomes
```

## Commands

```sh
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                     # needs a C toolchain; downloads PostgreSQL once
cargo run -p wb-server -- docs gen         # after changing CLI, errors or config keys
```

## Plan

Full plan, decisions and milestone definitions: `.kilo/plans/1789809743664-iterative-investigation-plan.md`, kept identical to the maintainer's Claude Code plan. Update both together.

Status: **M0 (foundation) delivered**, awaiting the maintainer's testing (`doc/demos/M0.md`). **M1 (knowledge + retrieval foundation) starts only on request.** Build one milestone at a time and stop after each one.
