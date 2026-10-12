# 0014. Possible features instead of a milestone roadmap

**Date:** 2026-10-10 · **Status:** Accepted

## Context

The plan was a sequence of milestones (M0–M5) and expansions (E1–E12). Each review reordered it; the sequence promised more than was known.

## Decision

Beyond the current milestone (M1: the store and SQL), the roadmap is an **unordered list of possible features**, chosen one at a time from what testing shows:

- **Investigations:** questions, hypotheses with predictions registered before the data, rounds that record their intervention, test plans with a recording recipe.
- **Metric definitions:** reusable SQL run identically on every recording, with the stored query as provenance; comparisons across rounds.
- **Hypothesis checks and an evidence ledger** (consistent, inconsistent, not testable; confirmatory or exploratory).
- **Hardware knowledge**, with its open problems ([0009](0009-hardware-knowledge-deferred.md)).
- **Similarity across recordings** (the unknown unknowns known elsewhere): recurrences, counterexamples, before and after a fix, outliers and novelty, behaviour modes, and the same problem described in other words. Exact vector search already works in DataFusion (`cosine_distance`); Lance adds approximate indexes, full-text and hybrid search. Embeddings need a local model. Statistical outliers and similar distributions are SQL recipes ([0020](0020-abnormality-and-similarity-as-sql-recipes.md)).
- **Images and point clouds in the store** (Lance blobs), for multimodal search.
- **More formats:** ROS 1 bags and ROS 1-encoded MCAP (no Rust crate decodes ROS 1 messages), LCM, MCAP protobuf and JSON, Rerun `.rrd`; CAN, DBC signal decoding (`can-dbc` parses DBC files but no crate decodes frames with them), ArduPilot DataFlash, MDF4, Betaflight; HDF5 and MATLAB, TDMS, WAV; PCAP; GNSS logs; Zarr, NumPy, TFRecord; plain-text logs.
- **Export** of a query result or a stream to Parquet, CSV or MCAP (any viewer opens MCAP). Sending to a running Rerun viewer is done ([0019](0019-send-query-results-to-rerun.md)).
- **Vortex** for the store, if bulk scans become the bottleneck ([0013](0013-lance-as-the-store-format.md)).
- **Time and clocks:** clock domains and mappings between clocks.
- **Extraction without an open session:** a daemon or scheduled extraction.
- **Background analysis and notices:** detectors, an inbox, hooks in the agent.
- **Plots** for the agent's vision, and captions describing signals.
- **Running generated code** in a sandbox.
- **Platforms and teams:** native Windows, a store shared by a team.
- **Causal-evidence tools:** contrast sets, dose-response, before and after views, confounder hints.
- **Skills** ([0006](0006-skills-deferred.md)).
- **An evaluation** against the same agent using generic tools.

## Alternatives

- **Keeping the milestone sequence:** it kept being reordered, and suggested commitments that were not decisions.

## Consequences

- Each next step is decided after testing the previous one, and recorded here as an ADR.
