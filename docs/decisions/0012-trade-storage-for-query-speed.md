# 0012. Trade storage for query speed: extract every recording in the background

**Date:** 2026-10-10 · **Status:** Accepted

## Context

The agent must be able to compute any metric quickly, across many recordings, and new files must become queryable without any command.

## Decision

Wise Bucket **extracts every recording** into a decoded, columnar copy (the store), in the background, while an agent session is open: new and changed files are found every minute, newest first, each in a low-priority child process. Images, point clouds, maps and video keep their timing and sizes only. Extraction pauses below 10 GB of free disk space.

## Alternatives

- **Index in place, decode on demand**, as the Foxglove data platform (indexes MCAP in your own bucket) and Rerun (`.rrd` chunks with an index) do: no copy, but the first query on each stream waits for decoding. These projects work at cloud scale, where copying petabytes is too expensive.
- **Convert per question**, as Bagel does (topics written to Arrow files for DuckDB on each question): the simplest store, but slow repeated questions and no query across many recordings.
- **Precomputed metrics:** see [0010](0010-metrics-are-sql-written-by-the-agent.md).

## Consequences

- Every query is fast, including across all recordings; the cost is disk space and background CPU.
- The store is derived: deleting it is safe, and a new extractor version re-extracts everything (`EXTRACTOR_VERSION`).
- Extraction needs an open agent session; a daemon or scheduled extraction is a possible feature ([0014](0014-possible-features-instead-of-milestones.md)).
