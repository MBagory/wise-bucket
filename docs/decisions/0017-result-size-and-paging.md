# 0017. Results capped by size; paging and downsampling are the agent's SQL

**Date:** 2026-10-11 · **Status:** Accepted

## Context

The `sql` tool returned at most 200 rows, a number nothing explained, so the agent could not plot or scan a long signal (a 200 Hz IMU over 10 minutes is 120,000 rows). The real limit is the agent's context: every returned row enters it, and clients cut tool results beyond about 25k tokens (Claude Code's default). Each row was also a JSON object repeating every column name.

## Decision

- **A result is capped by size** (about 60 KB, `MAX_BYTES`), not by a row count, and by `limit` rows when the agent sets one. Rows are arrays in the order of `columns`.
- When a result is cut, `truncated` is set and a **`hint`** says why and how to get the rest: aggregate, filter, or the condition for the next page (`t > <last t returned>`).
- **Paging and downsampling are SQL written by the agent** ([0010](0010-metrics-are-sql-written-by-the-agent.md)), taught by recipes: about 150 `date_bin` bins with min, avg and max to plot, pages by time window or by key on `(recording, t)`. The agent chooses every parameter; the server never writes SQL.

## Alternatives

- **Offset paging** (`OFFSET`, or a cursor holding a row number): each page re-runs the query and skips the rows before it, and pages shift when the background extraction adds a recording. It still works in SQL; the recipes do not teach it.
- **A cursor held by the server:** state per agent window with expiry rules, and unusable from `wisebucket sql` (one process per call).
- **A result materialized to a file, read page by page:** consistent and general, but a new kind of state to clean up. It belongs to the export feature ([0014](0014-possible-features-instead-of-milestones.md)), where a script reads the file instead of the agent's context.
- **A `downsample` option on `sql`:** it would duplicate `date_bin` and run SQL generated from names the agent chose, outside the locked-down path.
- **A typed `signal(stream, columns, from, to, bins)` tool:** no SQL to get wrong, but it cannot plot derived signals (power, a setpoint error, a join). Reconsider if agents fumble the SQL for simple fetches.

## Consequences

- The agent sees as many rows as fit, and is told how to page or aggregate when they do not.
- A page by key on `(recording, t)` skips rows sharing the same `t`; time windows do not. `stream()` exposes no row number to break ties.
- The result is still sent twice by the MCP library (structured content and its text copy); whether a client counts both against its limit is not yet checked.
