# 0032. Extraction checks in the notes

**Date:** 2026-10-26 · **Status:** Accepted (updates [0010](0010-metrics-are-sql-written-by-the-agent.md): a few checks are computed in advance)

## Context

A cross-check of the NMEA 0183 sample (`evals/crosscheck.sh`) found every value right, but the 16 s gap in a 0.5 Hz log came only from a `lag()` query the agent had to think of. In the ULog cross-checks (0024, 0026), a column of zeros was read as a missing barometer. The agent reads `recordings.notes` first; nothing there pointed at the timing or at flat signals.

Measured on every sample, checks on all streams and columns were noise: 155 to 589 constant float columns per ULog file, `t` going backwards in tables and ULog records, and 58 streams "stopping early" because records are stamped after the last sample.

## Decision

- **Checks run during extraction**, in the pass that writes each stream, and are written to the existing `notes`: per stream in `streams.notes`, a summary in `recordings.notes`.
- **Periodic streams only** for timing: at least 5 intervals, 80% of them within ±50% of the median.
- **Gap:** an interval over 5× the median and over 1 s.
- **Stops early:** a periodic stream ending more than 5% of the periodic streams' span, a gap and 1 s before the last periodic stream.
- **`t` going backwards** in a periodic stream.
- **`t - t_sensor`** varying by more than 1 s (unset stamps left out).
- **Flat stream:** more than 5 rows, every float column (nested fields included) constant or without values.
- Thresholds are constants in `crates/wb-data/src/checks.rs`; changing them bumps `EXTRACTOR_VERSION`.

## Alternatives

- **A separate `analysis` or `checks` column:** keeps file facts and heuristics apart, but adds a column the agent must be told about; the instructions say the checks are heuristics instead.
- **Computing at query time:** thresholds could change without extracting again, but every query of `recordings` would pay for it.
- **Column statistics (min, max, mean) and anomaly scores:** the fixed-statistics and stale-peers problems of 0010 and 0020.

## Consequences

- The agent sees gaps, early stops, clock steps and flat streams in its first query.
- Every recording is extracted again (extractor version 6).
- Notes now mix what the file says with what Wise Bucket computed.
