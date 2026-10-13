# 0024. Recording context from the files

**Date:** 2026-10-13 · **Status:** Accepted

## Context

A cross-check of the demo log `sample.ulg` against pyulog's own tools found every number Wise Bucket returned to be right. Still, an analysis made only with SQL reached three wrong conclusions, because the store kept the signals and dropped what the file says about itself:

- **The estimator:** an empty `ekf2_innovations` stream read as "the estimator is not running", while the parameter `SYS_MC_EST_GROUP = 1` selects LPE, not EKF2.
- **A limit:** a gyro inconsistency of 0.0865 rad/s read as "well under the default limit of 0.25", while `COM_ARM_IMU_GYR = 0.09` in that log.
- **The barometer:** its absence was inferred from zeros, while the log holds four errors "no barometer found on /dev/baro0".

The log's info (board `AUAV_X21`, firmware) and its 4 logger dropouts were missing too, and `recordings.start` showed 1970-01-01, because streams whose `timestamp` field is not set (0) counted in the minimum.

A survey of the other demo files found more of the same: default parameters in newer ULogs, a 52 KB crash dump and the boot console output in multi-part info, and the QoS of each ROS 2 topic in MCAP channel metadata and `.db3` topics.

The design criterion is the agent's: it must find this context in the places it already looks, and use it in few queries and few tokens.

## Decision

- **Short facts in a `properties` table** (`recording`, `key`, `value`), for every format: ULog info and MCAP metadata records (`<record>.<key>`). The MCAP `rosbag2` record (the bag summary) is left out: it repeats `recordings` and `streams`.
- **Multi-row content as streams with self-describing types.** A ULog file also gets `parameters` (`ulog parameters`), `logged_messages` (`ulog logged messages`), `dropouts` (`ulog dropouts`) and `texts` (`ulog texts`: boot console, crash dump and performance counters, one row per line). Older logs that number the lines of a text as info keys (`perf_counter_preflight-00`…) go to `texts` too.
- **pyulog's names and timing.** Records without a timestamp (parameters, parameter changes, dropouts, texts) take the largest data timestamp seen before them, starting from the header's. `parameters.initial` marks the values at the start of the log. Defaults are kept as `system_default` and `airframe_default`.
- **Notes that point to problems.** `recordings.notes`, in the agent's first query, counts the warnings and errors of every log stream (ULog `logged_messages`, ROS 2 `rcl_interfaces/msg/Log` topics, MAVLink `STATUSTEXT`) and quotes the first one; for ULog it also reports dropouts and crash dumps. One helper writes these notes for every format, so the absence of a count means nothing was logged.
- **Each source's own log levels** are kept: syslog names for ULog, 10–50 for ROS 2, `MAV_SEVERITY_*` for MAVLink.
- **A ULog recording starts at its header's time**, the start pyulog reports. Its end stays the last `t` over all streams, which is pyulog's last timestamp.
- **QoS in short** in `streams.qos` (`reliable, volatile, depth 10`), with the numeric values of older bags named.
- **Unset ULog timestamps and frozen times** are flagged in the stream's notes.

## Alternatives

- **A ULog-only `info` stream** instead of `properties`: no place for the key/values of other formats.
- **Cross-format `logs` and `parameters` views** with a normalized severity (as in the OpenTelemetry log model): useful only when recordings of different formats are compared, while peers are compared within one format.
- **ULog messages mapped onto `rcl_interfaces/msg/Log`**, as the Lichtblick viewer does: loses ULog's eight levels, and names a ROS topic that the file does not have.
- **Long texts as `properties` values:** paging a 52 KB crash dump 1,000 characters at a time, in a `recording.json` that every query reads.
- **The raw QoS YAML:** about 330 characters per topic, half of a result's budget in `SELECT * FROM streams` for a bag of 100 topics.
- **The bag summary YAML** (`metadata.yaml`, the MCAP `rosbag2` record): repeats `recordings` and `streams`.
- **Filling `hardware` from the ULog board (`ver_hw`):** the board is not the machine; ten drones can share one board model.

## Consequences

- Every recording is extracted again (`EXTRACTOR_VERSION` 3).
- A ULog file is read twice: once for the data, once for its records.
- `recording.json` grows by the short properties only; texts are Parquet streams.
- The agent instructions say to read the notes first, and the recording's properties and parameters before concluding.
- Context that only the engineer knows (the machine, the test, the conditions) is the next step: a `recordings.context` column and a `note` tool.
