# 0003. Managed PostgreSQL as the only store

**Date:** 2026-10-01 · **Status:** Superseded by [0011](0011-no-database.md)

## Context

Milestone M0 needed storage that several agent windows (several processes) can share, with full-text and fuzzy search, and later vector search (pgvector).

## Decision

Wise Bucket downloaded **PostgreSQL 17** (SHA-256-pinned prebuilt binaries from theseus-rs), ran it privately on a Unix socket, started it with the first agent session and stopped it after the last. Using your own PostgreSQL server was removed on 2026-10-01: managed mode was the only one.

## Alternatives

- **Your own PostgreSQL:** removed to keep one configuration to test and document.
- **SQLite:** considered later, when PostgreSQL itself was replaced ([0011](0011-no-database.md)).

## Consequences

- About 250 MB downloaded at setup, a platform floor (macOS 15+, glibc 2.34+), extra system libraries on Linux, and a start/stop lifecycle with locks and socket paths.
- These costs, with no data yet that needed a database, led to [0011](0011-no-database.md).

### Appendix: the pgvector provisioning spike (2026-09-27)

pgvector was first built from its pinned, checksum-verified source against the downloaded PostgreSQL (`make PG_CONFIG=… OPTFLAGS=""`, with `PG_SYSROOT=$(xcrun --show-sdk-path)` on macOS because PGXS records the build machine's SDK path), in about 14 s. A CI-built prebuilt binary later replaced the local build, then pgvector was removed altogether ([0005](0005-pgvector-and-embeddings-deferred.md)).
