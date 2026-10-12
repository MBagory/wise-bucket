# 0021. Parquet for streams

**Date:** 2026-10-13 · **Status:** Accepted (supersedes [0013](0013-lance-as-the-store-format.md))

## Context

Lance ([0013](0013-lance-as-the-store-format.md)) was chosen for the similarity features: vector indexes, versions, columns added without rewriting. The store's streams use none of them: they are written once, then scanned by column and time range. On the demo store, Lance took about 7 files per stream (124 files for 17 streams, 101 of them under 2 KB), refused dotted top-level field names (three of pyulog's six test logs failed to extract), and was the only reason the build needed `protoc`. Lance released a major version about every three weeks, all tied to the Arrow version of DataFusion. MATLAB does not read Lance.

Abnormality and similarity are SQL recipes ([0020](0020-abnormality-and-similarity-as-sql-recipes.md)); signal embeddings, if they come, fit in a list column read by an exact `cosine_distance` until about a million vectors.

## Decision

- **Each stream is one Parquet file** (`data/NNNN.parquet`), written with ZSTD compression and page statistics, pages of at most 8,192 rows: a time range skips the pages outside it.
- **Structs stay nested** and `stream()` flattens them into dotted columns at query time, as before. A column whose numeric type differs between recordings (message definitions change with firmware versions) is read as `Float64`.
- **DataFusion reads the files** with its own Parquet reader. `lance` is removed; the `parquet` crate shares DataFusion's Arrow version.

## Alternatives

- **Lance:** see the context. Reconsider for approximate vector search over more than about a million vectors, or for images and point clouds as blobs.
- **Vortex:** faster scans and random reads, and ALP compression for floats, but before 1.0: files written now are readable by later versions, not the reverse, and the library's API changes between releases (0.34 to 0.86 in six months). Reconsider after its 1.0 and its standardization as an Iceberg file format.
- **Arrow IPC:** the fastest to load, but larger files with no statistics.

## Consequences

- About one file per stream; the demo store's sizes are about the same as with Lance.
- Building needs no `protoc`; DataFusion, `parquet`, `re_ros_msg` and `rerun` are upgraded together for Arrow.
- pandas, Polars, DuckDB, Spark and MATLAB (`parquetread`) read the store.
- No vector indexes or versions in the store; `EXTRACTOR_VERSION` 2 extracts every recording again.
