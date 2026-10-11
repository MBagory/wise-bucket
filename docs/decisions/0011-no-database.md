# 0011. No database: columnar files queried by DataFusion

**Date:** 2026-10-10 · **Status:** Accepted (supersedes [0003](0003-managed-postgresql.md) and [0005](0005-pgvector-and-embeddings-deferred.md))

## Context

Everything M1 stores is **derived** from the recordings: it can be deleted and rebuilt by reading them again. A database server (PostgreSQL, [0003](0003-managed-postgresql.md)) brought a heavy install, a platform floor and a lifecycle, for data that needs neither transactions nor updates.

## Decision

**No database.** Each recording is extracted once into files (the store, [0012](0012-trade-storage-for-query-speed.md)), written once and made visible by an atomic rename. **Apache DataFusion**, a query engine embedded in Wise Bucket, runs the agent's SQL over them. Several agent windows coordinate with operating-system file locks, which the system releases if a process dies. PostgreSQL and its code are removed.

State that cannot be rebuilt (a future investigations feature) can likely live in Lance tables too, since every commit gets a version, which gives an order. SQLite remains the fallback if it needs constraints or many small writes.

## Alternatives

- **Keeping PostgreSQL:** the strongest lockdown (a separate login role) and a path to a shared team database, at the cost of the managed install.
- **SQLite for the catalog:** transactions between windows, but one more storage form for data that is derived anyway.
- **DuckDB as the only engine:** only one process may write a DuckDB file, and every agent window is its own process.
- **Lance or Delta Lake tables for everything:** transactional table formats, not needed for files written once.

## Consequences

- Setup is instant; nothing runs when no agent session is open.
- Installing now needs no download, but building from source needs a C compiler and `protoc`.
- No text indexes: names are matched by scanning, which is fast for thousands of streams.
