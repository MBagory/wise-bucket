# 0013. Lance as the store's format

**Date:** 2026-10-10 · **Status:** Superseded by [0021](0021-parquet-for-streams.md)

## Context

The store needs a columnar file format that DataFusion queries efficiently, and that suits the similarity features Wise Bucket aims at: finding the unknown unknowns known elsewhere through embeddings ([0014](0014-possible-features-instead-of-milestones.md)).

## Decision

Streams are stored as **Lance** datasets (`lance` 13, file format 2.2 set explicitly), with structs kept nested (Lance column names cannot contain dots) and flattened into dotted names at query time, and a zone-map index on the time column `t`.

**Pins:** `datafusion` 54.1, `lance` 13 and `re_ros_msg` 0.38 share one Arrow version (58); they are upgraded together.

## Alternatives

- **Vortex** (Linux Foundation incubation): the fastest scans and random reads, chosen first, then reversed. Its open-source format has no vector, full-text or scalar indexes, it changes weekly before 1.0, and its community is about a tenth of Lance's (in October 2026, about 0.2 M monthly PyPI downloads against 3.4 M for `pylance`).
- **Parquet:** universal and stable, but no vector indexes, versions or column backfill for the similarity features.
- **Arrow IPC:** fastest to load, but larger files with no statistics.

## Consequences

- Ready for similarity: vector, full-text and scalar indexes, embedding columns added later without rewriting data, versions.
- pandas (`pylance`), Polars, DuckDB and Hugging Face read the store; MATLAB does not (an export is a possible feature).
- Slower bulk scans than Vortex; a heavier dependency, with a major version about every three weeks.
- One writer function and one registration function: moving to Vortex or Parquet would be one change.
