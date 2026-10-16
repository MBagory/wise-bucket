# 0029. CAN signals from the DBC files the engineer confirms

**Date:** 2026-10-16 · **Status:** Accepted

## Context

A CAN log holds frames: an identifier and payload bytes. Their signals (speeds, temperatures, states) are defined in a DBC file, which the log does not contain. A log's folder may hold no DBC, one, or several (one per bus); the DBC may also sit in a parent folder or outside the data roots. Decoding a frame with the wrong DBC gives wrong numbers, not errors. [0016](0016-formats-need-an-external-rust-decoder.md) waited for a crate that decodes frames with a DBC.

## Decision

- **Which DBC:** the context key `dbc` ([0025](0025-context-from-the-engineer.md)) names the DBC files of the CAN logs it covers: `root:path` or an absolute path, separated by `;`, each optionally bound to one channel (`2=root:path`), or `none`. Until it is set, a log's recording note says what to ask the engineer: to confirm the `.dbc` found next to the log (else in its nearest parent folder inside the root), to choose among several, or where it is. The agent records the answer on the most general target it holds for, so later logs are decoded without asking.
- **What is stored:** the raw `frames`, always. With confirmed DBCs, also one stream per DBC message, `<dbc>/<message>`: `channel`, one column per signal (physical value), and `<signal>_label` for signals with a value table. A recording note counts the frames whose identifier no DBC defines.
- **Decoder:** `falcon_mdf`, with its `dbc` feature: `can-dbc` parses the DBC, `falcon_mdf` decodes the frames.
- **Re-extraction:** a CAN log's DBC choice, and the sizes and times of its DBC files, are part of its fingerprint: editing a DBC or the key `dbc` extracts the log again. The `note` tool starts a pass at once.

## Alternatives

- **`dbc-rs`:** it decodes frames, but rejected the four real vehicle DBCs tried (comma.ai opendbc: overlapping signals, senders missing from the node list, a keyword it does not read), and caps messages and signals per message at build time.
- **Every DBC found decodes the log, without asking:** a frame decoded with another bus's DBC gives wrong signals, silently.
- **Decoded streams only:** a wrong DBC (scale, offset, byte order) could not be checked against the bytes, and bus load could not be computed.

## Consequences

- The first question about a CAN log may be a question for the engineer, asked once per folder or root.
- CAN logs are stored twice, as frames and as signals ([0012](0012-trade-storage-for-query-speed.md)).
- On the four DBCs tried, `falcon_mdf` decodes what cantools decodes. On a frame whose multiplexer value the DBC does not define, it keeps the other signals, where cantools fails the whole frame.
