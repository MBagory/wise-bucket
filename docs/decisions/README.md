# Decision records

Why Wise Bucket is built the way it is. Each **architecture decision record** (ADR) states one decision: its context, what was chosen, the alternatives set aside, and the consequences.

ADRs are **never rewritten**. When a decision changes, a new ADR supersedes the old one, which stays here, marked with what replaced it. To propose one, copy the [template](template.md) and take the next number.

The principles these decisions add up to are summarized in the [README](../../README.md#overview).

| # | Decision | Date | Status |
| --- | --- | --- | --- |
| [0001](0001-local-stdio-mcp-server.md) | A local stdio MCP server; the agent owns the model | 2026-09-28 | Accepted |
| [0002](0002-native-rust-log-readers.md) | Native Rust log readers; no Python readers, no Bagel | 2026-09-28 | Accepted |
| [0003](0003-managed-postgresql.md) | Managed PostgreSQL as the only store | 2026-10-01 | Superseded by [0011](0011-no-database.md) |
| [0004](0004-user-wide-only.md) | User-wide only; recordings organized by data root | 2026-10-02 | Accepted |
| [0005](0005-pgvector-and-embeddings-deferred.md) | pgvector and embeddings deferred | 2026-10-03 | Superseded by [0011](0011-no-database.md) |
| [0006](0006-skills-deferred.md) | Skills deferred | 2026-10-03 | Accepted |
| [0007](0007-no-release-before-the-mvp.md) | No release before the MVP; install from source | 2026-10-03 | Superseded by [0015](0015-first-release-after-m1.md) |
| [0008](0008-hardware-rather-than-robot.md) | "Hardware" rather than "robot" | 2026-10-05 | Accepted |
| [0009](0009-hardware-knowledge-deferred.md) | Hardware knowledge deferred, with its open problems | 2026-10-06 | Accepted |
| [0010](0010-metrics-are-sql-written-by-the-agent.md) | Metrics are SQL written by the agent; nothing computed in advance | 2026-10-10 | Accepted |
| [0011](0011-no-database.md) | No database: columnar files queried by DataFusion | 2026-10-10 | Accepted (supersedes [0003](0003-managed-postgresql.md) and [0005](0005-pgvector-and-embeddings-deferred.md)) |
| [0012](0012-trade-storage-for-query-speed.md) | Trade storage for query speed: extract every recording in the background | 2026-10-10 | Accepted |
| [0013](0013-lance-as-the-store-format.md) | Lance as the store's format | 2026-10-10 | Accepted |
| [0014](0014-possible-features-instead-of-milestones.md) | Possible features instead of a milestone roadmap | 2026-10-10 | Accepted |
| [0015](0015-first-release-after-m1.md) | First release after M1 is tested | 2026-10-10 | Accepted (supersedes [0007](0007-no-release-before-the-mvp.md)) |
| [0016](0016-formats-need-an-external-rust-decoder.md) | A format ships only when an external Rust crate decodes it | 2026-10-11 | Accepted |
| [0017](0017-result-size-and-paging.md) | Results capped by size; paging and downsampling are the agent's SQL | 2026-10-11 | Accepted |
| [0018](0018-extraction-on-file-events.md) | Extraction starts on file events | 2026-10-11 | Accepted |
| [0019](0019-send-query-results-to-rerun.md) | Query results sent to the Rerun viewer | 2026-10-13 | Accepted |
