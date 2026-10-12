# 0020. Abnormal and similar recordings found with SQL recipes

**Date:** 2026-10-13 · **Status:** Accepted

## Context

The engineer wants to find recordings that differ from their peers (the same stream and column in other recordings) and recordings that resemble each other, before knowing what to look for. Engines built on DataFusion score anomalies in SQL with window functions (GreptimeDB's `anomaly_score_zscore`, `anomaly_score_mad` and `anomaly_score_iqr`); none runs ML inside the query engine. DataFusion 54 already provides what these scores need: `median()` works as a window function, also over a sliding frame (it can retract rows), and `array_distance` and `cosine_distance` compare vectors.

## Decision

- **Abnormality and similarity are SQL written by the agent** ([0010](0010-metrics-are-sql-written-by-the-agent.md)), taught by three recipes in the `sql` tool's description:
  - **Peers:** one value per recording (p99, rate, maximum gap, standard deviation…), then a robust z-score across recordings: `(v − median) / (1.4826 × MAD)`, with `median() OVER ()` in two steps.
  - **Spikes:** the same score inside one recording, over a sliding frame (`ROWS BETWEEN 200 PRECEDING AND CURRENT ROW`).
  - **Similar:** a vector of quantiles per recording (`make_array` of p05 to p95), ranked by `array_distance` to a reference recording; it does not depend on when things happen in a recording.
- **The agent chooses the peers and the columns** for each question: comparable recordings (the same hardware, the same kind of test) and meaningful columns (not counters, sequence numbers, timestamps, enums, or quaternion components one by one). A score of 3 or more reads "unusual against these peers".
- **Nothing is added to the store or to the SQL engine.** The `ulog` demo set grows to six PX4 logs so that the recipes have peers to work on.

## Alternatives

- **Column statistics saved at extraction** (percentiles, rate, gaps per column): instant scores across many recordings, and the base for notices pushed to the agent, but the same fixed summaries [0010](0010-metrics-are-sql-written-by-the-agent.md) rejected (noise on counters, enums and quaternions; phases mixed). Reconsider when the recipes take more than about 5 s across the store, or with background notices ([0014](0014-possible-features-instead-of-milestones.md)).
- **`anomaly_score_*` window functions**, ported from GreptimeDB (Apache-2.0): shorter queries, but the recipes compute the same scores. Reconsider if agents get the recipes wrong.
- **A fixed comparison tool:** it cannot choose peers per question ([0010](0010-metrics-are-sql-written-by-the-agent.md)).
- **Signal embeddings first** (Chronos-Bolt, MOMENT, Mantis, run locally): they see shapes that quantiles miss, but cost a model, a background pass and stored vectors, and do not explain their distances. Reconsider when the recipes miss known shape problems; the statistics are then the baseline to beat.

## Consequences

- Any column, any peers and any threshold, with the query behind each score.
- The agent must write three-step queries; the recipes give their shape.
- Scores across many recordings scan their streams at query time.
- Shape similarity (the same event at different times, a different pattern with the same distribution) is not covered.
