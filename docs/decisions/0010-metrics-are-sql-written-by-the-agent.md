# 0010. Metrics are SQL written by the agent; nothing computed in advance

**Date:** 2026-10-10 · **Status:** Accepted

## Context

A first M1 design pre-computed metrics for every recording: statistics per field and per fixed time window (1 s, growing with the recording's length). Reviewing it showed the flaws: fixed windows are too coarse for short events and not comparable between recordings, the statistics are fixed in advance, means of quaternion components or enums are noise, and the precomputation scales with data, not with questions.

## Decision

**Nothing is computed in advance.** The agent writes a read-only SQL query for each question: rates, gaps (`lead()`), windows (`date_bin`) at the question's time scale, percentiles, joins between streams, comparisons across recordings. The same queries run with `wisebucket sql`.

The SQL is locked down by construction: only the store's tables are registered, DDL, writes and other statements are refused, and each query is capped in time, memory and returned rows.

## Alternatives

- **Precomputed summaries and windows:** see the context.
- **A fixed metrics tool:** computing a chosen list of statistics on demand would bring back the fixed-statistics problem.
- **Generated code (Python) run by Wise Bucket:** deferred; SQL covers most metrics and can be locked down.

## Consequences

- Any metric can be computed, at any time scale, and the query behind each number can be shown.
- Metric definitions reused across recordings, with their query as provenance, are a possible feature ([0014](0014-possible-features-instead-of-milestones.md)).
