# 0036. Recipes only for Wise Bucket's dialect and schema

**Date:** 2026-11-05 · **Status:** Accepted (updates the recipes of [0017](0017-result-size-and-paging.md) and [0020](0020-abnormality-and-similarity-as-sql-recipes.md))

## Context

The `sql` tool's description carried 13 recipes (3,290 characters), loaded in every session. The 11 cross-check reports of 2026-10-25 and 10-26 (`evals/crosscheck.sh`) show what the agent needs:

- **Analysis it already knows.** No report asks for analysis recipes (windows, percentiles, peer scores, spikes, similar distributions) or for column statistics.
- **Wise Bucket's dialect and schema it does not.** `t - lag(t)` is a duration that cannot be divided or compared to a number: queries were lost to it in ros2-mcap, dataflash and mdf4, with the Gaps recipe in the description, because it did not show the conversion.
- **Precomputed facts that point at problems** saved the most: `recordings.notes` gave in one query what took several scripts ([0032](0032-extraction-checks-in-notes.md)).

## Decision

- **A recipe exists only for what the model cannot know: Wise Bucket's dialect or schema**, in one line. Three remain:
  - **Gaps (in seconds):** `t` as nanoseconds with `CAST(t AS BIGINT)`;
  - **Plot:** about 150 `date_bin` bins with min, avg and max ([0017](0017-result-size-and-paging.md));
  - **Page by time window** ([0017](0017-result-size-and-paging.md)).
- **The method of [0020](0020-abnormality-and-similarity-as-sql-recipes.md) stays as a guideline** in the agent instructions: one value per recording, scored as (value − median) / (1.4826 × MAD) against peers of the same machine and kind of test, with the peers named.
- **The `sql` description states the time rule:** differences of `t` are durations; `CAST(t AS BIGINT)` gives nanoseconds.
- **A test caps the length** of the `sql` description (5,029 characters + 2,000) and of the agent instructions (5,483 + 2,000). Recipes over tables still to come qualify when they teach their schema.

## Alternatives

- **No recipes:** the paging and plot rules would remain as guidelines, but the time conversion is best shown by a working query.
- **Per-column statistics computed at extraction** (a `columns` table): no report asked for them, and whole-recording statistics mix the phases of a test.
- **Hints in query errors** for known dialect failures: costs nothing until a query fails, but each failure costs a call. Reconsider if a failure keeps showing in the cross-checks.

## Consequences

- The `sql` description shrinks from about 7,500 to 5,029 characters.
- The agent writes peer scores, spikes and distribution comparisons itself.
- A recipe or table description that grows past the cap fails the build until room is made.
