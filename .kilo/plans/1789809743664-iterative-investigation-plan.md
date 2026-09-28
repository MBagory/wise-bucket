# Wise Bucket: Plan

> **Status (2026-09-28):** **M0 (foundation) is delivered** and waiting for the maintainer's own testing ([doc/demos/M0.md](../../doc/demos/M0.md)). **M1 starts only on request.**
>
> **Two identical copies:** this plan is kept identical in `.kilo/plans/1789809743664-iterative-investigation-plan.md` (Kilo Code) and `~/.claude/plans/read-the-plan-in-twinkling-micali.md` (Claude Code). Update both together. The repository's `AGENT.md` summarizes it for coding agents.

## 1. Goal

Help a robotics R&D engineer turn unexpected robot behaviour into **evidence-backed findings** and the **next useful experiment**, across several rounds:

```text
question → hypotheses → physical test → recording → analysis → updated hypotheses → next test …
```

The AI harness's LLM brings broad technical knowledge. Wise Bucket brings **memory** (the robot world, the investigation, what each recording contains) and **measurements with provenance** (numbers computed from the actual recordings).

**Benchmark:** does it improve investigations compared with **the same harness and model with generic tools**? This is measured at the M5 evaluation gate.

## 2. Decisions

| Topic | Decision |
| --- | --- |
| Architecture | A local **Rust stdio MCP server**, `wise-bucket-server`. The **harness** (Claude Code, Cline, Kilo…) owns the LLM, BYOK keys, token budgets and the conversation loop. **The server never calls an LLM.** |
| Store | **PostgreSQL 17 + pgvector ≥ 0.8**, either **managed** by Wise Bucket (the default) or **brought by the user** (`WB_DATABASE_URL`). Bulk signal data goes in Parquet files, never in Postgres. |
| Search | **Embedding generation, storage and search are a key feature** from M1: local multilingual embeddings, with hybrid semantic + full-text + trigram search. |
| Log readers | **Native Rust** (ROS 2 MCAP first; other formats after the gate). No Bagel, no Python readers. |
| Focus | **Context, metrics and interpretations attached to each log**, linked to the robot knowledge and to questions and hypotheses. |
| Generated code | Running LLM-generated code is **deferred** (E6). Numbers come from locked-down SQL plus built-in Rust operators. |
| Order | **Golden path 1** (question-first) → **M5 evaluation gate** → golden path 2 (background analyzer) and the rest of the expansion, reordered by the gate. |
| Boundaries | The engineer declares **data roots** (the only readable folders). **No MCP tool can add a root.** Roots and round scope replace the original plan's grants and budgets. |
| Code layout | Four crates: `wb-core` and `wb-server` exist now; `wb-data` comes in M3, `wb-exec` later. |
| Platforms | Linux and macOS. Windows runs through **WSL2**; native Windows comes later (E7). |
| Docs | First-class from day one: mdBook, a classic open-source README, FAQ and troubleshooting. A milestone isn't done until its docs are. |
| License | **Apache-2.0.** |
| Delivery | **One milestone at a time.** Development stops after each one so the maintainer can test it. |
| Deferred | The VS Code extension (its own plan), server-driven automatic continuation, research tools, export, team mode. |

## 3. Milestones at a glance

| # | Milestone | What you get | Value | Success is defined as | Status |
| --- | --- | --- | --- | --- | --- |
| **M0** | Foundation | An installable binary. `setup` provisions a local Postgres + pgvector and declares **data roots**. `init` configures a robot repo for the harness. `server_info` and `doctor` answer. Docs site, CI | Proves the chain install → harness → MCP → database | From a clean macOS/Linux machine, following only the docs, `server_info` works in Claude Code and lists your roots. `doctor` is clean. Two windows work at once. Postgres stops after the last one. BYO Postgres works | ✅ delivered, awaiting user testing |
| **M1** | Knowledge + retrieval foundation | The robot world (robots, sensors, buses, software). Unknown things are asked about and remembered, then found again by alias, typo, paraphrase or French/English | You stop re-explaining your robot to the LLM | "lidar" unknown → questions asked → answers persist. "the Ouster", "front laser scanner" and "lidar avant" resolve to one entity. Stale facts are flagged. Recall@5 meets its threshold | next, on request |
| **M2** | Investigations | Questions, hypotheses with testable predictions, and data requests turned into recording recipes | A resumable plan instead of a chat that disappears; you know what to record next | The lidar/arm scenario yields discriminating predictions, a `ros2 bag record` recipe and a reported gap. A paraphrased question finds the earlier investigation | planned |
| **M3** | Logs (ROS 2 MCAP) | Attach a bag. Topics are indexed and bound to components. Context is attached and topics get searchable captions | Recordings become evidence tied to your robot | The fixture decodes exactly. Topics auto-bind; unknown topics trigger questions. "which stream measures arm current?" finds it | planned |
| **M4** | Metrics, interpretations, search | Traceable metrics, interpretations, hypothesis checks, cross-round comparisons, annotated plots, full hybrid search, read-only SQL | Answers backed by traceable, comparable numbers; closes golden path 1 | H1 `consistent`, H2 `not_testable` on the fixture. The round-2 change is visible. SQL lockdown holds. Retrieval thresholds are met | planned |
| **M5** | Evaluation gate | 3–5 real investigations run with and without Wise Bucket | A go/adjust/stop decision before expanding | A written report and a reprioritized expansion list | planned |
| E1–E10 | Expansion | Clocks, formats, importers, golden path 2, code execution, Windows, adapters/daemon, team mode, TS-LLM captioner | Ordered by M5 | Defined when scheduled | later |

## 4. How it is used

### Golden path 1: question-first (M1–M4)

*"Why does the lidar drop scans when the arm moves fast?"*

1. **Frame.** `investigation_create(question)`, then `knowledge_resolve(["lidar", "arm"])`. An unknown mention comes back with the questions for its kind. The agent asks, and `knowledge_upsert` persists the answers with their provenance.
2. **Hypothesize.** `hypothesis_add` records each hypothesis with **predictions**: an observable (entity + quantity + window) plus an expected metric outcome. The predictions are chosen to discriminate between hypotheses.
3. **Request data.** `data_request_create` turns the predictions into a recording recipe (`ros2 bag record …`, the procedure, the duration) through the entity ↔ stream knowledge. Gaps (e.g. "no voltage topic known") are reported, asked about, and persisted.
4. **Attach.** `log_attach` (inside a root) → describe → streams bound to entities → `log_context_set` (robot, configuration, procedure, conditions).
5. **Measure.** `metric_compute` runs locked-down SQL with built-in operators and stores the metrics on the log. `hypothesis_check` returns `consistent`, `inconsistent` or `not_testable`, each with its assumptions and counter-checks. **Only the engineer concludes.**
6. **Interpret and iterate.** `interpretation_add` attaches interpretations to the log. The next round is compared with `metrics_compare`. A new conversation resumes with `investigation_get(since_revision)`.

### Golden path 2: data-first background analyzer (E5)

Attach or drop data → a background pipeline (describe, bind, profile, detectors) → anomaly metrics and interpretations → associations with open hypotheses, predictions and data requests, plus analogies with past cases → **notices**.

The server cannot post into the harness chat on its own, because MCP has no "wake the agent" mechanism. Interpellation therefore uses:
- **An agent- and OS-agnostic core:**
  - `inbox_list` and `inbox_ack` tools
  - a `_wb.pending_notices` summary on every tool result
  - server `instructions`, an `inbox` prompt, and a `wb://inbox` resource with change notifications
- **Optional adapters (E8):** Claude Code `SessionStart`/`UserPromptSubmit` hooks, desktop notifications, and a daemon so analysis runs with no chat open.

### Every log carries its own context, metrics and interpretations

| Attached to a log | What it holds | Provenance |
| --- | --- | --- |
| **Context** | Robot and configuration (firmware, params), procedure, conditions, operator notes, stream ↔ component bindings, time fields and clock notes | `embedded` / `inferred` / `imported` / `user_stated` (relayed by the agent, optional verbatim `quote`) / `agent_proposed` |
| **Metrics** | A value, unit, stream/field, window and method (SQL with `wb-ops` functions, or a description) | `computed` (with an execution record: SQL, file version, code version) or `reported` (method required). The two are never mixed silently. |
| **Interpretations** | An observation, anomaly, candidate explanation or assessment. Cites metrics and selections, and links to hypotheses | `model_hypothesis` / `user_stated`. Never promoted automatically. |
| **Selections / POIs** | A time range plus a note (e.g. the docking window) | as above |

Across logs you can search by robot, entity, context or metric condition, compare a metric across rounds, and see an investigation's log timeline.

## 5. Configuration and data roots (implemented in M0)

| File | Owner | Holds |
| --- | --- | --- |
| **User config**: Linux `~/.config/wisebucket/config.toml`, macOS `~/Library/Application Support/wisebucket/config.toml` | the engineer, per machine (`setup`, `roots add`) | state directory, `[database]` mode and `keep_running`, **data roots** (absolute paths; NAS mounts allowed) |
| **Project config**: `<robot-repo>/.wisebucket/config.toml` | the team (committed) | `project`, `default_robot`, repo-relative roots |
| **Harness config**: `<robot-repo>/.mcp.json` (Kilo: `.kilocode/mcp.json`) | `init` | the launch command: absolute binary path, `serve --project <repo>` (plus `--state-dir`/`--config` when given explicitly). No data paths, no secrets |

- **Precedence:** CLI flags > `WB_*` environment variables (`WB_CONFIG`, `WB_STATE_DIR`, `WB_RUNTIME_DIR`, `WB_DATABASE_URL`, `WB_PROJECT_DIR`, `WB_KEEP_RUNNING`) > project config > user config > defaults.
  - Every effective value tracks its **origin** (`config show --origin`).
  - Unknown keys are rejected.
  - Edits keep comments (`toml_edit`).
- **Secrets never go into config files.**
  - The managed database passwords are generated and stored in `<state>/secrets/db.toml` (mode 0600, directory 0700).
  - An external URL comes only from `WB_DATABASE_URL` or `--database-url`, and passwords are masked in output.
  - For external mode, `init` writes `"env": {"WB_DATABASE_URL": "${WB_DATABASE_URL}"}` so the harness passes it through.
- **Data roots:** a named folder (`[a-z0-9][a-z0-9_-]{0,31}`) that Wise Bucket may read. Nothing outside a root can be attached.
  - Logs are referenced as `root:relative/path` (e.g. `bags:2026-09-24/dock_run3`). The database will store `(root, relative path)`, so moving a mount only means updating the root's `path`.
  - Paths are canonicalized. Symlinks must stay inside a root. **Overlapping roots are both rejected.** Case-insensitive file systems are handled (macOS).
  - **Project roots** must be relative and stay inside the repository.
  - **There is no MCP tool to add a root.** `log_attach` outside a root fails with `outside_roots`, and the error explains how to add one.
  - The server reads its roots when it starts, and `server_info` lists them with their origin. A harness with shell access could still edit the files; the docs say so plainly.
  - `roots check` counts candidate recordings: a rosbag2 folder (`metadata.yaml`) counts as one recording, plus loose `.mcap`, `.bag` and `.ulg` files.
- **Commands:**
  - `setup` (interactive, or `--yes --root name=path`)
  - `init [--robot --name --root --harness claude|kilo|cline --merge --print]`
  - `roots add|list|remove|check [--in-project]`
  - `config show [--origin]|path`
  - `db status|start|stop [--force]`
  - `backup <file>`, `restore <file> --yes`
  - `doctor`
  - `docs gen [--check]`
  - a global `--json` flag for machine output

## 6. Reading logs (Rust only, M3+)

```text
log file (read-only, inside a root)
  │ 1. detect   magic bytes / layout → format (MCAP first)
  │ 2. index    summary/index only: channels, schemas, counts, start/end, chunk index (no payload decode)
  │ 3. decode   on first use of a stream, in a child process with memory limits:
  │             chunk → decompress (zstd/lz4) → dynamic CDR decoder from the embedded ros2msg definition
  │             → Arrow RecordBatches (~64k rows) → time-sorted Parquet (zstd, row-group statistics)
  │ 4. query    DataFusion registers the Parquet (locked down); wb-ops functions compute metrics
  ▼
Parquet cache (derived, evictable) + Postgres rows (metadata, metrics, embeddings)
```

- **Formats:** every reader implements the `SourceReader` trait and produces Arrow with flattened dotted columns (`linear_acceleration.z`), int64-ns time columns, and the original time fields kept.
  - **M3:** ROS 2 MCAP.
  - **After the gate:** rosbag2 `.db3`, CSV (with a mapping), PX4 ULog, CAN (candump/ASC/BLF + DBC), ArduPilot DataFlash / MAVLink tlog, ROS 1, MCAP protobuf/JSON, then Betaflight and MDF4.
- **Robustness:**
  - Nothing is loaded whole into RAM.
  - Every length field and decompression is capped, and recursion is bounded.
  - The decoder runs in a child process, and each parser has a fuzz target.
  - Correctness is checked against fixture manifests, MCAP conformance files, and differential tests against reference tools (CI only).
- **Time:**
  - int64 ns end to end.
  - M3 reports time fields and obvious clock issues (a boot-time `header.stamp`, drift, resets) as warnings.
  - Full clock domains and mappings, with the "unresolvable" rule, come in E1.
- **Bounded outputs:** every listing is paginated and summarized. A privacy setting redacts configured fields (e.g. GPS) from tool results.

## 7. Storage

### Postgres modes and lifecycle (implemented in M0)

| Mode | How |
| --- | --- |
| **Managed** (default) | `setup` downloads **PostgreSQL 17.11.0** from [theseus-rs/postgresql-binaries](https://github.com/theseus-rs/postgresql-binaries), **SHA-256-pinned per target** (x86_64/aarch64 macOS and Linux). It then **builds pgvector 0.8.6** from its pinned, checksum-verified source against those binaries (PGXS; `PG_SYSROOT` fixed on macOS; `OPTFLAGS=""` for portability; about 15 s), and runs `initdb` (SCRAM passwords). `setup` bootstraps the `wb_app` role (login), the `wb_reader` role (nologin, granted to `wb_app`), the `wisebucket` database, and the `vector` + `pg_trgm` extensions. The server listens **only on a Unix socket**, in `<state>/run`, or `/tmp/wb-<hash>` if the path is too long (with an owner check). There is no TCP listener. |
| **Bring your own** | PostgreSQL ≥ 15 with pgvector ≥ 0.8 and `pg_trgm`. The user owns the database; a superuser creates the extensions. Tested against `pgvector/pgvector:pg17`. |

- **Lifecycle:**
  - The first MCP session starts Postgres (about 2 s). Each session keeps at least one pooled connection with `application_name = 'wisebucket'`.
  - When a session ends, it counts the remaining session backends in `pg_stat_activity` under a cross-process file lock (`<state>/lifecycle.lock`), and stops Postgres if none are left.
  - `keep_running` disables the stop.
  - CLI commands use `application_name = 'wisebucket-cli'`, and stop Postgres only if they started it.
- **Migrations:** plain-SQL `sqlx` migrations embedded in the binary. Concurrent servers are serialized by sqlx's advisory lock.
- **Backup:** `pg_dump -Fc` of the `wb` and `wb_views` schemas plus `_sqlx_migrations` (extensions belong to the superuser). `restore --yes` runs `pg_restore --clean --single-transaction`. Recordings and the Parquet cache are never part of a backup.
- **Planned:** `db upgrade` for PostgreSQL major-version changes (dump + restore), and CI-built pgvector binaries as a fallback for machines without a C toolchain.

### State directory

```
<state-dir>/   Linux ~/.local/share/wisebucket · macOS ~/Library/Application Support/wisebucket
  runtime/     PostgreSQL + pgvector (shareable via WB_RUNTIME_DIR)       [M0]
  pg/          PostgreSQL data directory                                   [M0]
  secrets/     db.toml (0600)                                              [M0]
  run/         Unix socket directory (0700)                                [M0]
  logs/        server.log (daily), postgres.log                            [M0]
  models/      embedding / reranker models (sha256-pinned)                 [M1]
  cache/decoded/<log_version_id>/<stream>.parquet                          [M3]
  results/<execution_id>.parquet · cards/ · exports/                       [M4]
```

### Schema

**Conventions:**
- Everything lives in schema `wb`; read-only views for the LLM live in `wb_views`.
- ids are `uuid` v7. Signal times are `bigint` ns; wall-clock times are `timestamptz`. Flexible values are `jsonb`.
- Every row carries `rev` (from `wb.rev_seq`), `session_id` and `created_at`.
- **Semantic rows are append-only:** a correction supersedes the old row and keeps the history.

**Implemented (M0):**
- the `vector` and `pg_trgm` extensions
- schemas `wb` and `wb_views`
- sequence `wb.rev_seq`
- table `wb.session` (client name/version, pid, host, cwd, project, `wb_version`, started/last_seen/ended)
- grants to `wb_reader` when that role exists

**Planned:**

| Milestone | Tables |
| --- | --- |
| M1 | `entity`, `entity_alias` (trigram index), `fact` (validity, `last_confirmed`), `relation`, **`search_doc`** (subject, filter columns, contextualized `body`, generated `tsv`, `embedding halfvec(384)`, `model_id`), `embedding_model`, `job` (`FOR UPDATE SKIP LOCKED` + lease) |
| M2 | `investigation`, `round`, `round_log`, `hypothesis`, `prediction` (observable + `metric_def` + expected comparator), `data_request`, `entry` (journal, incl. `session_summary`) |
| M3 | `log`, `log_version` (root-relative path, size, mtime, file id, quick/full hash, status `registered/indexed/verified/stale`), `stream`, `binding_rule`, `stream_binding`, `context_item`, `selection` |
| M4 | `metric_def` (SQL template over stream *roles*), `metric`, `execution`, `interpretation`, `link` (`cites/supports/contradicts/about`), the `wb_views.v_*` views |

## 8. Knowledge model (M1)

- **Entities:** an open kind taxonomy (robot, subsystem, sensor, actuator, payload, compute, bus, protocol, software_node, firmware, power, test_procedure), with names and aliases.
- **Facts:** value, unit, provenance, confidence, a validity window (configuration changes) and `last_confirmed`.
- **Relations**, and **stream bindings** to recordings.
- **Seed ontology (YAML):** common components, their typical attributes, **the questions to ask for each kind**, and message-type → kind hints.
- **Hybrid resolver:** exact alias → trigram fuzzy match → **semantic nearest entities** → kind hints → stream names. It returns `resolved`, `ambiguous` (candidates with scores) or `unknown` (with questions).
- **Reviewability:** `knowledge_list(stale_only)` flags old, unconfirmed or conflicting facts. `knowledge_export` writes Markdown/YAML for human review. Upserts supersede and keep the history.
- **Importers (E4):** URDF, parameter YAML, DBC.

## 9. Embeddings and search (M1–M4)

- **What is embedded:** every searchable item gets a `search_doc` row in the same transaction:
  - entities and facts
  - streams (name, type, fields, bound entity)
  - context items, interpretations, hypotheses and predictions, data requests
  - questions, journal entries, metric definitions
- **Contextualized text:** each doc carries its context, e.g. `[robot: Rover-B] [log: dock_run3] [entities: front lidar, arm] observation: scan gaps co-occur with arm current > 8 A`.
- **Generation, local:**
  - **Default:** `fastembed-rs` (ONNX Runtime) with `intfloat/multilingual-e5-small` (384 dimensions, French/English).
  - **Options:** `multilingual-e5-base` or `bge-m3` for more quality, **model2vec-rs** for small machines, and an optional cross-encoder reranker.
  - Models are fetched by `setup`, sha256-pinned, with an offline bundle.
  - **Pipeline:** the write stores the doc with an immediate tsvector → an `embed` job → a batch worker fills in the vectors. Search works at once through FTS and trigram, and becomes semantic seconds later.
  - Each vector records its `model_id`; changing the model triggers a background re-embed.
- **How the LLM searches:**
  1. **`search`**: a single hybrid SQL query. It combines pgvector **HNSW** KNN (`halfvec_cosine_ops`, `hnsw.iterative_scan` for filtered queries), full-text search (`websearch_to_tsquery` + `ts_rank_cd`) and `pg_trgm` name similarity, fused with **Reciprocal Rank Fusion**, plus the optional reranker. It returns snippets, scores, why each item matched, and ids.
  2. **`wb_sql`**: read-only SQL over documented `wb_views` views, as the `wb_reader` role, with `BEGIN READ ONLY`, `statement_timeout`, a `work_mem` cap and row caps. The lockdown is enforced by the database.
  3. **Markdown cards**: one per log, investigation and entity. A read-only projection, regenerated through `LISTEN/NOTIFY`, and grep-able by any harness.
- **Retrieval is tested:** `fixtures/retrieval/` holds paraphrases, jargon and acronyms, and French/English pairs. CI reports **recall@5 and MRR** and fails on regressions.

## 10. Describing signals in words

| Layer | Produced by | Trust | When |
| --- | --- | --- | --- |
| **1. Deterministic captions** (backbone) | Rust templates over describe facts, `wb-ops` features, detectors, bindings and context, e.g. *"IMU on boat Neptune, `/imu` 100 Hz, 42 min… spike 6.3 MAD at 12:04:31.220"*. Stored as a computed context item and **embedded**, so text search finds signals | Exact and traceable | M3 (describe facts), M4 (features, spikes) |
| **2. Plots for the harness's vision** | `plot_render` returns an annotated PNG (`plotters`); the harness LLM describes it, stored as a `model_hypothesis` | A model reading, to check against metrics | M4 |
| **3. Signal embeddings** | Handcrafted feature vectors first, then time-series foundation-model encoders (MOMENT, Chronos-2) through ONNX if they win on the eval. They serve "looks like this" search and analogies | Similarity only, not aligned with text | E5 |
| **4. Time-series language models** | ChatTS or OpenTSLM behind a local model server, opt-in; numbers cross-checked against layer 1 | LLM output, labeled `model_hypothesis` | E10 |

## 11. Sessions and concurrency

- **Three kinds of session:**
  - The **chat transcript** belongs to the harness. Wise Bucket never stores chat text.
  - The **MCP session** is one `wb.session` row per server process. It records the client from `initialize` and a heartbeat every 30 s, and it is ended on exit. It is the provenance of every write.
  - The **investigation state** in Postgres is the durable memory. It survives new conversations and different harnesses.
- **Handover:**
  - Server `instructions` ask the agent to write an `entry(kind=session_summary)` when a conversation ends.
  - The next conversation resumes with `investigation_get(since_revision)`.
- **Concurrency:**
  - Several harness windows share one Postgres, using MVCC.
  - Jobs are claimed with `SKIP LOCKED` plus leases, so any session can process any job.
  - `LISTEN/NOTIFY` propagates changes across sessions.
  - Files are published atomically.
  - No daemon is required.

## 12. Architecture

### Crates

```
crates/
  wb-core     [M0] error catalog (codes + fixes → docs), config layering + origins, roots, paths, fsutil
              (atomic/private writes, file locks), db (managed provisioning/lifecycle, connect, requirements,
              migrations, backup), session
              [M1+] knowledge, investigations, log records, retrieval (search_doc, embedder, hybrid search)
  wb-server   [M0] CLI (clap) + MCP server (rmcp 3.4, stdio), commands, logging (stderr + file; stdout = MCP)
  wb-data     [M3] formats (detector, SourceReader, MCAP), rosmsg (msg/idl, CDR), schema → Arrow, Parquet cache,
              DataFusion lockdown, wb-ops operators, time profile
  wb-exec     [E6] reserved for executing generated code
docs/ (mdBook) · doc/demos · doc/spikes · crates/wb-server/tests (acceptance tests) · fixtures/ [M3]
```

Split a crate only when its boundary actually hurts.

**Dependencies:**
- **M0 (in use):**
  - MCP and runtime: `rmcp` 3.4 (server, macros, stdio), `tokio`
  - Storage: `sqlx` 0.9 (Postgres, rustls-ring, uuid, chrono, json, migrate)
  - Downloads and archives: `ureq` 3, `sha2`, `hex`, `flate2`, `tar`, `getrandom`
  - Configuration and paths: `toml`, `toml_edit`, `serde`, `directories`, `dunce`
  - CLI and logging: `clap`, `clap-markdown`, `dialoguer`, `tracing`, `tracing-subscriber`, `tracing-appender`
  - Toolchain: MSRV 1.94 (required by sqlx 0.9), toolchain pinned to 1.98.1
- **Planned:** `pgvector`, `fastembed` (ort), `model2vec-rs` (optional), `serde_yaml`, `strsim` (M1); `mcap`, `arrow`, `parquet`, `datafusion`, `rustfft`, `blake3`, `plotters` (M3–M4)

### MCP tool contract

| Milestone | Tools |
| --- | --- |
| M0 ✅ | `server_info` (version, database status or a structured error, session and client, state dir, project, default robot, roots with origin, root problems) |
| M1 | `knowledge_resolve`, `knowledge_upsert`, `knowledge_get`, `knowledge_search`, `knowledge_list`, `knowledge_export` |
| M2 | `investigation_create/get/append`, `round_start`, `hypothesis_add/update`, `data_request_create`, prompt `investigate` |
| M3 | `log_attach`, `log_describe` (paginated), `log_context_set`, `stream_bind`, `selection_add`, `stream_caption` |
| M4 | `metric_compute`, `metric_record`, `interpretation_add`, `log_summary`, `logs_search`, `metrics_compare`, `hypothesis_check`, `query`, `plot_render`, `search`, `wb_sql`; resources `wb://investigation/{id}`, `wb://log/{id}` |
| E5 | `inbox_list`, `inbox_ack`, prompt `inbox`, resource `wb://inbox` |

Every error is a structured result with `code`, `message`, `fix` and `docs_ref`. The server still starts when the database is unavailable, so `server_info` can explain the fix from inside the harness.

### Documentation

- **mdBook in `docs/`, organized by Diátaxis:**
  - philosophy
  - get started
  - how-to (from M1)
  - reference: **generated** by `docs gen` for errors, CLI and configuration
  - FAQ, troubleshooting
- **Kept true by CI:**
  - `docs gen --check`
  - every error code documented (a test)
  - tutorials have executable twins
  - `mdbook build`, `typos`, `lychee`
- **Shared source:** the server `instructions` come from a docs fragment.

## 13. Milestone details

### M0 – Foundation ✅ delivered (2026-09-27)

**What exists:**

| Area | Delivered |
| --- | --- |
| Binary | `wise-bucket-server` with `serve` (default), `setup`, `init`, `roots`, `config`, `db`, `backup`, `restore`, `doctor`, `docs gen`, and a global `--json` |
| Database | Managed PostgreSQL 17.11.0 + pgvector 0.8.6 (pinned, verified, built locally); BYO mode; start with the first session, stop after the last; `keep_running`; migration `0001_foundation.sql`; sessions with client info and heartbeat; backup and restore |
| Configuration | User and project TOML plus environment and flags, with origins; data roots with validation and candidate counts; `init` for Claude Code (`.mcp.json`), Kilo (`.kilocode/mcp.json`) and Cline (a printed snippet), never overwriting (`--merge` to add) |
| Diagnostics | 24 documented error codes; `doctor` (configuration, state dir and free space, Postgres, pgvector, socket, database, roots, project, harness config) |
| Docs | Introduction; philosophy (context, principles, how it works, trust and privacy); get started (install incl. WSL2, configure, connect your harness); generated references (CLI, configuration, errors); FAQ; troubleshooting; `doc/demos/M0.md`; `doc/spikes/pgvector-provisioning.md` |
| Repository | A classic open-source README; LICENSE (Apache-2.0); CODE_OF_CONDUCT (Contributor Covenant 2.1); CONTRIBUTING (DCO); SECURITY; CHANGELOG; CITATION.cff; AGENT.md; issue and PR templates; dependabot; CODEOWNERS (@MBagory); `ci.yml` (fmt, clippy, tests on ubuntu + macos-14, the BYO job with a pgvector service container, docs, cargo-deny); `docs-publish.yml`; `.editorconfig`, `rustfmt.toml`, `clippy.toml`, `deny.toml`, `typos.toml`, `lychee.toml`, `.gitattributes`, `.gitignore` |
| Tests | 21 `wb-core` unit tests and 2 `wb-server` unit tests. Nine acceptance tests drive the real binary with a real rmcp client: two sessions and stop-after-last, `keep_running`, starting without setup (structured error), roots, doctor codes, backup/restore, the "connect your harness" twin, BYO Postgres (with `WB_TEST_DATABASE_URL`), and generated docs up to date |

**Changes from the original M0 plan:**
- **No `postgresql_embedded` crate.** The same theseus archives are downloaded directly, which gives control over the socket-only setup, the pgvector build into the same installation, and cross-process locks. pgvector is **built from source at setup** (spike outcome); CI-built binaries remain the fallback.
- **No `figment`.** An in-house loader with origin tracking is simpler and exact.
- **A file lock plus `pg_stat_activity`** decides "stop after the last session", instead of a Postgres advisory lock.
- **Roles, the database and extensions are created by `setup` as superuser.** The migration only grants to `wb_reader` if that role exists, so BYO works without CREATEROLE.
- **`roots add --in-project`**, because `--project` clashed with the global `--project <dir>`.
- **Backups exclude extensions**, which belong to the superuser.
- The book uses a text architecture diagram (no Mermaid plugin); the README keeps Mermaid.

**Verified** on an Intel Mac (macOS 15.7):
- `setup` takes about 1 minute including downloads.
- The MCP chain works over stdio, and the client name is recorded.
- All tests pass, including BYO against `pgvector/pgvector:pg17`.
- `fmt` and `clippy -D warnings` are clean.
- `mdbook build` works, and the release `cargo install --locked` produces a 12.8 MB binary.

**Not yet verified:**
- Linux and macOS arm64: CI has not run yet because the repository isn't published.
- The interactive prompts of `setup` and `init`, which need a TTY.
- A real Claude Code session: the tests used an rmcp client and raw JSON-RPC.
- `cargo-deny`, `typos` and `lychee`: CI only.

**How the maintainer tests it:** follow [doc/demos/M0.md](../../doc/demos/M0.md), about 10 minutes, and collect feedback before M1.

### M1 – Knowledge and the retrieval foundation

**Scope:**
- Entities, facts (provenance, validity), relations, the seed ontology with question templates.
- `search_doc` + the embedder (fastembed + multilingual-e5-small, fetched by `setup`) + the embed job worker (with the `job` table) + HNSW, GIN(tsv) and trigram indexes + model versioning.
- The hybrid resolver.
- The `knowledge_*` tools.
- `fixtures/retrieval/` with a recall@5/MRR report in CI.

**Docs:** `teach-your-robot`, the ontology reference (generated), and a how-to on embeddings and models.

**Done when:**
- "lidar" is unknown → questions are asked → answers are upserted → after a restart it resolves with provenance.
- "the Ouster", "front laser scanner" and "lidar avant" resolve to one entity.
- Stale and conflicting facts are flagged.
- Recall@5 meets its threshold.

### M2 – Investigations, hypotheses, data requests

**Scope:**
- Investigations, rounds, hypotheses with predictions, data requests with ROS 2 recipes and gap reporting, all embedded.
- The `investigate` prompt.
- `search` over past questions and hypotheses.

**Docs:** the `first-question` tutorial and its executable twin.

**Done when:**
- The lidar/arm scenario produces discriminating predictions, a `ros2 bag record` recipe and a reported gap, all persisted.
- A paraphrased question retrieves the earlier investigation.

### M3 – Logs: ROS 2 MCAP

**Scope:**
- The M3 tables.
- `log_attach` (roots, identity, versions).
- `wb-data`: the MCAP reader (index first, decode on demand), `rosmsg` CDR, Arrow + Parquet cache, the decode child process.
- `log_describe` with time warnings, `stream_bind` (rules, message types and semantic similarity, confirmed through the agent), `log_context_set`, `selection_add`.
- Stream captions v1.
- Fuzz targets.

**Docs:** `first-log`, `formats/ros2-mcap`, `record-good-data`.

**Done when:**
- The fixture decodes to the manifest's values.
- `/scan` and `/imu` auto-bind, and an unknown topic triggers questions.
- "which stream measures arm current?" returns the right topic.
- The context persists.

### M4 – Metrics, interpretations, full search

**Scope:**
- The DataFusion lockdown and `query`.
- `wb-ops` (`rate`, `gaps`, `mad_spikes`, `rolling_std`, `fft_peak`, `xcorr_lag`, …), each checked against reference values.
- The M4 tables.
- `metric_compute/record`, `interpretation_add`, `log_summary`, `logs_search`, `metrics_compare`, `hypothesis_check`.
- The session-summary handover.
- Stream captions v2 and `plot_render`.
- The full hybrid `search`, `wb_views` + `wb_sql`, and the Markdown cards.

**Docs:** `context-metrics-interpretations`, a how-to on searching your investigations, the views reference, and a FAQ entry on the check statuses.

**Done when:**
- The fixture gives H1 `consistent` and H2 `not_testable`.
- `metrics_compare` shows the round-2 change.
- Write, DDL and `pg_read_file` attempts through `wb_sql` are rejected by Postgres.
- Retrieval thresholds are met, including signal-caption queries.
- `plot_render` output is viewable in Claude Code, and its interpretation is stored with provenance.

### M5 – Evaluation gate

**Scope:**
- Take 3–5 **real** cases, each with expected findings recorded beforehand. Use no resolutions in the model input.
- Compare Claude Code + Wise Bucket with **the same model and generic tools** on:
  - time to the first useful test
  - evidence preparation effort
  - time to a supported conclusion
  - wrong or unsupported claims
  - unnecessary experiments
  - clarification burden
  - resumption across sessions
  - install-to-first-use friction
  - the usefulness of the stored context, metrics and interpretations
- An LLM grader alone is not enough.

**Done when:** `doc/eval/` holds a written go/adjust/stop decision and a reprioritized expansion list.

### Expansion (ordered by M5)

- **E1 – Time and clocks.**
  - Clock domains per stream and time field, inferred from `header.stamp` vs `log_time` offsets, drift and resets.
  - `time_mapping` records with uncertainty, from a user statement, a shared event, or an in-file reference (ULog GPS UTC, DataFlash GPS time).
  - A mapping is never taken from the lag that maximizes correlation.
  - A lag below the uncertainty is "unresolvable".
- **E2 – Tier 1 completion:** rosbag2 `.db3` (with a `msg_definitions` prerequisite when definitions are missing) and CSV (with a mapping).
- **E3 – Formats, each its own milestone:** PX4 ULog, CAN + DBC, ArduPilot/MAVLink, ROS 1, MCAP protobuf/JSON, then Betaflight and MDF4. Each gets fixtures, differential tests and fuzzing.
- **E4 – Knowledge importers:** URDF, parameter YAML, DBC.
- **E5 – Golden path 2:**
  - a background pipeline (in-session plus catch-up)
  - the agnostic inbox
  - detectors with dedup, thresholds and "not useful" feedback
  - associations with open hypotheses
  - analogies over signal embeddings
- **E6 – Execution of LLM-generated code.** The decision is pending; the options were studied:
  - Pyodide on Deno (WASM, no Docker, cross-platform; numpy/scipy/pandas/pyarrow/duckdb available; 4 GB limit)
  - a native venv (no boundary; interactive only)
  - containers (Docker/Podman)
  - microVMs (microsandbox)
- **E7 – Native Windows:** an MSVC pgvector build, loopback TCP plus ACL-protected secrets, Windows paths, Windows CI.
- **E8 – Optional adapters:** Claude Code hooks, desktop notifications, a daemon (launchd/systemd), Unix-socket/named-pipe IPC.
- **E9 – Team mode:** a shared Postgres, workspaces, access control.
- **E10 – Optional time-series LLM captioner** (see Describing signals in words).

## 14. Risks

- **Decoder correctness is the foundation.** Mitigations: fixture manifests, conformance files, differential tests, fuzzing.
- **Parser effort in Rust is large.** Formats come after the gate, ordered by demand.
- **Relayed facts can be wrong or stale.** Mitigations: provenance, staleness flags, export for review, supersede with history. Check statuses never claim proof.
- **Dependency churn:** the MCP spec and `rmcp`, sqlx, DataFusion. Pin versions; accept DataFusion's build time.
- **Postgres operational weight:** a managed process, socket and disk use, major upgrades. Mitigations: the lifecycle, `db` commands, `backup`/`restore`, `doctor`, troubleshooting docs.
- **Install size:** about 250 MB now (PostgreSQL, pgvector, an empty cluster); M1 adds ONNX Runtime and a model of about 100–500 MB. Mitigations: an offline bundle and the model2vec option.
- **The pgvector build needs a C toolchain.** Clear `toolchain_missing` guidance; CI-built binaries are the fallback.
- **Embedding quality on jargon and short texts.** Mitigations: hybrid search, contextualized text, a reranker, a retrieval eval that gates changes.
- **Privacy:** tool results reach the LLM provider through the harness. Results are bounded, a redaction setting covers sensitive fields, and embeddings stay local.
- **Harness bypass:** the harness's own shell can read any file. Wise Bucket enforces only its own tools, and the docs say so.

## 15. Verification strategy

- **Every milestone:**
  - `cargo fmt`, `clippy -D warnings`, `cargo test --workspace`
  - acceptance tests driving the real binary with an rmcp client
  - executable docs twins
  - `docs gen --check`
  - a `doc/demos/Mx.md` guide
  - a manual Claude Code session
- **Database tests:**
  - managed mode against a shared runtime (`WB_TEST_RUNTIME_DIR`)
  - BYO mode against a `pgvector/pgvector:pg17` container
  - from M1: concurrent servers, `SKIP LOCKED` job claiming, lease recovery, the `wb_reader` lockdown
- **From M1:** the retrieval eval (recall@5, MRR) fails CI on regression.
- **From M3:** reader tests against fixture manifests, MCAP conformance files, `cargo fuzz` smoke runs.
- **M5:** the evaluation report.

## 16. Open items

- **Repository:** <https://github.com/MBagory/wise-bucket>. The docs site will be <https://mbagory.github.io/wise-bucket/> once GitHub Pages is enabled.
- **Publish and run CI:** this validates Linux and macOS arm64, `cargo-deny`, `typos` and `lychee`.
- **Release binaries:** e.g. `cargo-dist`. Installation is from source until then.
- **The maintainer's M0 test:** the interactive prompts and a real Claude Code session.
- **Pending decisions:** code execution (E6), and whether to commit `.mcp.json` in robot repositories (it contains a machine-specific binary path).

## 17. Rationale and sources

This part condenses the original investigation, which predates the Rust/Postgres decisions. The findings below still shape the design:

- **MCP cannot wake an idle chat.**
  - Tools are invoked by the client.
  - Resource notifications don't trigger model turns.
  - `sampling/createMessage` is optional, controlled by the client, and doesn't post into the visible chat.

  Hence the harness drives the loop, and golden path 2 uses an inbox surfaced through tool results, instructions and optional hooks. Sources: [MCP sampling](https://modelcontextprotocol.io/specification/2025-11-25/client/sampling), [MCP resources](https://modelcontextprotocol.io/specification/2025-11-25/server/resources).
- **A server enforces only its own tools.** The harness's shell and file tools are outside its control. Hence engineer-declared roots, no root-adding tool, and honest documentation.
- **Investigation contract:** frame → inspect → scope (roots) → investigate → recommend the next measurement → resume with a new round → conclude or stay unresolved. Round inputs are immutable; the conversation is not the database.
- **Statistical caution:**
  - A correlation-maximizing alignment is not a physical delay.
  - Closed loops, common causes, multiple testing and cherry-picked windows mislead.

  Hence the `consistent / inconsistent / not_testable` statuses, counter-checks, and the clock-mapping rules.
- **Scale is unknown:** benchmark on design-partner data. Never load whole files. Report cold and warm timings separately.
- **Evidence semantics:** tool observations, model hypotheses, external claims and engineer assessments are labeled separately, and nothing is promoted automatically.

**Sources checked during planning:**
- [pgvector](https://github.com/pgvector/pgvector) (0.8 iterative index scans)
- [theseus-rs PostgreSQL binaries](https://github.com/theseus-rs/postgresql-binaries)
- [MCAP spec](https://mcap.dev/spec)
- [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [SQLx](https://github.com/launchbadge/sqlx)
- [fastembed-rs](https://github.com/Anush008/fastembed-rs), [model2vec-rs](https://github.com/MinishLab/model2vec-rs)
- [Pyodide packages](https://pyodide.org/en/stable/usage/packages-in-pyodide.html)
- [Anthropic sandbox-runtime](https://github.com/anthropic-experimental/sandbox-runtime), [Codex sandbox analysis](https://simonwillison.net/2025/Nov/9/codex-sandbox-investigation/), [microsandbox](https://github.com/superradcompany/microsandbox)
- [ChatTS](https://github.com/NetManAIOps/ChatTS), [OpenTSLM](https://github.com/OpenTSLM/OpenTSLM), [MOMENT](https://github.com/moment-timeseries-foundation-model/moment), [THEMIS](https://arxiv.org/pdf/2510.03911), [VisualTimeAnomaly](https://mllm-ts.github.io/)
- [Bagel](https://github.com/Extelligence-ai/bagel), evaluated and not used
- [OWASP SSRF prevention](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html), for the future research tools
