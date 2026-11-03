# Spike: how Sift groups runs into families

**Date:** 2026-11-03 · **Outcome:** people define Sift's families, and nothing clusters runs. A saved query proposes candidates, and a person includes or excludes each one.

## Question

Wise Bucket compares a recording with its peers, chosen by the agent for each question ([0020]). Sift, a telemetry platform for hardware tests, groups runs into **families**. Does it form them automatically from the data, or does the user define them, and with what?

## What was read

- Sift's public API, [`sift-stack/sift`] at `cfcb67f` (2026-11-02): [`families.proto`], and where rules, rule evaluation, reports, runs and automations use it. The Families service first appeared there on 2026-07-20.
- Sift's documentation ([group runs into a family], [compare runs against a family baseline]) and [blog post], through search excerpts only: `docs.siftstack.com` and `www.siftstack.com` were outside the allowed network of the session that ran this spike.

## Findings

A Sift run is a time range of data from one or more assets (the machines): the counterpart of a recording. A family is a named set of nominal runs of one kind, used as the baseline for new runs.

- **Membership is a list kept by people.** Each run of a family is included or excluded (`is_exclusion`), with a rationale, who decided, when, and in which version. A family is created with at least one run.
- **A query only proposes.** `run_query`, a CEL filter over runs, "is only for reference, and does not affect the runs that are included in the family". Runs that match it and are neither included nor excluded are **candidates**, counted and listed for review, then included or excluded in bulk. An excluded run is not proposed again.
- **The query sees what is declared about a run, never its signals:** asset, name, description, tags, asset tags, `metadata["key"]` (string, number or boolean), start and stop times, duration, annotation states. Every Sift channel belongs to an asset, so the machine is always known.
- **Nothing adds runs by itself.** Automations, fired by a stopped run (matched by asset, name or metadata) or by a schedule, only run evaluations. No part of the public API clusters runs or suggests members by similarity, and no documentation excerpt mentions it.
- **Every change is a version.** A version has a generated change message and the user's notes, and a family imports and exports as YAML. A report records the family version its rules were judged against.
- **Members are aligned before they are compared.** An alignment is a named T-0: the run's start or end, an annotation by name (first, last, every or nth occurrence; its start or end), or a timestamp. A member can override it; a member without a point is left out of that alignment.
- **Statistics per channel, in windows around a T-0.** A family stat names a channel (with fallback names for members that call it otherwise), a sample rate, and AVG, MEDIAN, MIN, MAX, STDEV, SUM or INPUT_COUNT; ±1, 2 and 3 sigma bands come with AVG and STDEV. A range is a window around an alignment ("Startup ± 5 s"). Stats are computed for each family version.
- **Rules flag a new run outside the envelope.** A rule refers to a family stat as a variable (`$A`: family, stat, expression, range), and the run under test gets its own alignment points.
- **Next to families:** tags and metadata on assets and runs (what candidate queries filter on), and campaigns, which group the reports of many runs for a review program (status, not statistics).

## What it means for Wise Bucket

| Sift | Wise Bucket |
| --- | --- |
| Run | Recording (`root:path`) |
| Asset | `context['hardware']` ([0025]) |
| Tags and metadata | `context` keys; `properties` from the files ([0024]) |
| Candidate query (CEL) | A `WHERE` on `recordings`, `streams` and `properties` |
| Members included or excluded, with a rationale, versioned | Not modeled |
| Alignment on an annotation | Not modeled: the `events` table planned in [0025] |
| Family stats and rules, computed for each family version | The Peers and Similar recipes, at query time ([0020]) |

- **A family is narrower than peers.** Peers are every comparable recording, outliers included, and the median and MAD tolerate some. A family holds nominal runs only: grown automatically, it would absorb the anomalies it should flag, and its sigma bands would widen. That is why a person confirms each member.
- **Proposals must come from the files.** Sift proposes from metadata that ingestion always sets. Recordings on disk rarely say which machine made them (`--hardware` was empty in practice, [0025]), so proposals would come from what the files hold: the same set of streams, the board and firmware in `properties`, the folder.
- **Statistics stay queries.** Envelopes stored per family would be precomputed answers, which [tenet 3] rules out. An envelope across aligned members can be a recipe, reconsidered at [0020]'s threshold of about 5 s.

## Follow-ups

- An ADR on families. The cheapest form is a `family` key written with `note` ([0025]): a folder for the bulk, a recording for an exception. `dbc` already sets the conventions it needs: `;` between several values, and `none` to take one recording of a folder out, since an empty value would bring back the folder's. It keeps no rationale and no history, which Sift has.
- A recipe that proposes members, run by the agent and shown to the engineer before `note`.
- Alignment on an event once `events` exists; until then, members align on their start.

[0020]: ../decisions/0020-abnormality-and-similarity-as-sql-recipes.md
[0024]: ../decisions/0024-recording-context-from-the-files.md
[0025]: ../decisions/0025-context-from-the-engineer.md
[tenet 3]: ../tenets.md#3-decode-once-query-fast
[`sift-stack/sift`]: https://github.com/sift-stack/sift
[`families.proto`]: https://github.com/sift-stack/sift/blob/cfcb67f19f21c4bd41a69b42152343dafd7fbb02/protos/sift/families/v1/families.proto
[group runs into a family]: https://docs.siftstack.com/documentation/analyze/families/group-runs-into-a-family
[compare runs against a family baseline]: https://docs.siftstack.com/documentation/analyze/families/compare-runs-visually-against-a-family-baseline
[blog post]: https://www.siftstack.com/mission-critical/codify-your-best-runs-as-the-baseline
