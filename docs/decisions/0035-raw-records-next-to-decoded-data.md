# 0035. Raw records next to decoded data

**Date:** 2026-11-04 · **Status:** Accepted (adds the first [engineering tenet](../tenets.md))

## Context

The cross-checks of the demo samples (`evals/crosscheck.sh`) found data a reader library dropped before Wise Bucket saw it, so no note could count it: an ASC remote frame written without a length and the `CAN n Status` lines (`blf_asc`), and NMEA 2000 fast-packet frames that made no message (`canboat`, only in its log). They also found a decoded value with no way to check it: a `.db3` bag whose stored definition of `rcl_interfaces/msg/Log` is cut short gives a `/rosout` level of 251 instead of 20, and nothing showed the bytes it came from. The agent of the cross-check decoded those records by hand from the files.

## Decision

- **A decoded row keeps its record** in the column `raw`, as the file holds it: the line for text formats (NMEA 0183, NMEA 2000, ASC), the bytes for binary ones (ROS 2 and MCAP messages, MAVLink records), which the `sql` tool shows in hex. Images, point clouds and other messages over 64 KiB keep their size only.
- **The records that gave no decoded row** go to the stream `raw`: `t` when they have one, `position` (line number, byte offset or message number), `kind` (`comment`, `header`, `Statistic`, `junk`, the topic of a rejected message…), and `text` or `bytes`.
- **`recordings.notes` counts them by kind** (`20 line(s) not read as frames, kept in stream raw: header 7, Statistic 6, …`).
- **Coverage:** for these formats, decoded rows and `raw` rows hold every line, or every byte, of the file once; the sample tests check it.
- Formats whose library hides its records (BLF, ULog, ArduPilot, MDF4, tables) stay without `raw` until the library exposes them.
- A new first tenet: **nothing lost in decoding**.

## Alternatives

- **Counting dropped lines only**, in the notes: shows that something is missing, not what.
- **A raw stream per format** (`lines`, `objects`, `records`) beside the decoded streams, with a `decoded` flag on every record: four names for one idea, and every decoded record stored twice.
- **Parsing ASC lines in Wise Bucket** instead of with `blf_asc`: against [0016](0016-formats-need-an-external-rust-decoder.md); ASC frames are matched to their lines by time instead.
- **A check of leftover payload bytes** for ROS 2 definitions: needs `re_ros_msg` to report the bytes it consumed; the `raw` column shows them meanwhile.

## Consequences

- An agent can check any decoded value against its record, and read what the decoder left out, with SQL.
- These streams take about twice the disk space; ASC lines need one more pass over the file.
- A line `blf_asc` drops with the same time as the next frame can take that frame's line: line numbers from `blf_asc` would make the match exact.
- Every recording is extracted again (extractor version 8).
