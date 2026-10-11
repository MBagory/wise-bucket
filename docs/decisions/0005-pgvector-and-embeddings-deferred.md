# 0005. pgvector and embeddings deferred

**Date:** 2026-10-03 · **Status:** Superseded by [0011](0011-no-database.md)

## Context

pgvector was installed at setup in M0, but nothing used it before similarity search, planned for milestone M4.

## Decision

pgvector, and all embeddings (text and signal), were deferred to M4. Until then, search used full-text and trigram matching only.

## Alternatives

- **Keeping pgvector installed from M0:** a cost (download, build or prebuilt binaries) with no use.

## Consequences

- PostgreSQL was then removed altogether ([0011](0011-no-database.md)). Vector search is now a possible feature, with exact search available in DataFusion and approximate indexes in Lance ([0013](0013-lance-as-the-store-format.md), [0014](0014-possible-features-instead-of-milestones.md)).
