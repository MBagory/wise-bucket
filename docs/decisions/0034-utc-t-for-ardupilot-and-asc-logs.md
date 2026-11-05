# 0034. UTC `t` for ArduPilot and ASC logs

**Date:** 2026-10-29 · **Status:** Accepted

## Context

The cross-checks of the ArduPilot and CAN samples (`evals/crosscheck.sh`) found two time axes an agent misread:

- An ArduPilot log with a GPS fix started at the UTC time of boot (2017-10-29), while every `t`, and so its `end`, stayed time since boot (1970): `end - start` was negative.
- An ASC log's `t` counted from the start of the measurement (1970 dates), while its `date` header (`Sam Sep 30 15:06:13.191 2017`) was only in `properties`. The agent read the stream note as a claim of absolute time.

`blf_asc` parses the ASC date, German months included, but in the time zone of the machine running Wise Bucket: the same file would get other times on another machine.

## Decision

- **ArduPilot:** with a GPS fix (the time base of `dflog`), `t` is the UTC time of boot plus `TimeUS`, and the UTC time of boot is the property `boot_utc`. Without a fix, `t` stays time since boot. Either way `start` is the first timed record: format and parameter records come before it without a time, and a log saved from the middle of a flight would otherwise seem to start at boot (the cross-check of the demo log: 82.8 s instead of 1.05 s).
- **ASC:** `t` is the `date` header, read as UTC, plus each frame's time. Wise Bucket parses the date itself: weekday skipped, English or German month, 24-hour or `am`/`pm`. When it can't, `t` counts from the start of the measurement, and the stream note says so.
- The ASC stream note says the date is the logging PC's local time, read as UTC, as BLF start times already are.

## Alternatives

- **Time since boot for the ArduPilot `start` too, with the UTC time of boot in `properties`:** the same axis as ULog, but aligning a log with other recordings would take arithmetic in every query.
- **The absolute timestamps of `blf_asc`:** the server's time zone would change the data.
- **Asking the engineer for the ASC time zone (a `note` key):** exact, but one more question per folder of logs. It can be added later without changing this decision.

## Consequences

- ArduPilot logs with a GPS fix and ASC logs line up with other recordings in UTC; an ASC log is off by its PC's offset from UTC.
- Times before 2000 still mean time since boot, or since the start of an ASC measurement whose date can't be read.
- Every recording is extracted again (extractor version 7).
