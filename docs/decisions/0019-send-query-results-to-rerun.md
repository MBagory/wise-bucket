# 0019. Query results sent to the Rerun viewer

**Date:** 2026-10-13 · **Status:** Accepted

## Context

To show a recording's signals in Rerun, the agent went around Wise Bucket: a `sql` result holds about 60 KB ([0017](0017-result-size-and-paging.md)), and there was no way to hand the store's rows to a viewer. It decoded the original file again with Python and wrote an `.rrd` file.

## Decision

- **A `rerun` tool** (and `wisebucket rerun "<query>"`) runs one read-only `SELECT` in the same locked-down session as `sql` (30 s, 1 GB) and streams **every row** to a running Rerun viewer over gRPC (`rerun+http://127.0.0.1:9876/proxy` unless `url` is set). Rows go to the viewer, not to the agent's context: there is no size cap.
- **The input is a query**, written by the agent ([0010](0010-metrics-are-sql-written-by-the-agent.md)): raw streams, derived signals and joins all go through it.
- The result must have a timestamp column `t`. **Every timestamp column becomes a Rerun timeline** under its own name (`t`, `t_sensor`), so the times the agent finds with SQL are the times in the viewer. A timeline whose first value is before 2000 counts from boot (PX4 ULog, MAVLink) and is sent as durations.
- **Every numeric or boolean column becomes a scalar series** (`Scalars`) at `<entity>/<column>`; SQL aliases and the optional `entity` prefix name them. Rows with a null value or a null `t` are left out of that series. Other columns are skipped and listed in the result.
- Every send goes to one Rerun recording, application and recording id `wisebucket`, so series sent by several calls can be shown together.
- The Rerun SDK (`rerun` crate, `sdk` feature only) uses the same Arrow version as DataFusion and Lance ([0013](0013-lance-as-the-store-format.md)).

## Alternatives

- **The whole recording as input** (every stream, every column): one call shows a full log, but no derived signals, and hundreds of series the engineer did not ask for. It can be added later as a loop over this tool if agents need many calls to show one log.
- **An `.rrd` or MCAP file**, opened by the viewer: works without a running viewer, and an MCAP file would also open in Foxglove, Lichtblick and PlotJuggler. It adds files to clean up; it belongs to the export feature ([0014](0014-possible-features-instead-of-milestones.md)).
- **Timelines named like Rerun's MCAP importer** (`message_log_time`, `message_publish_time`): Wise Bucket's `t` is the MCAP log time, but it is also the time of ULog, tlog and table files; keeping the SQL names lets the agent use the same name in both tools.

## Consequences

- The agent shows any signal of the store in Rerun in one call, at full rate.
- Sending the same query twice adds its rows twice to the `wisebucket` recording.
- The original file opened in Rerun next to it is a separate recording: for MCAP, its `message_log_time` has the values of Wise Bucket's `t`.
- Images, point clouds, lists and text are not sent.
- Other viewers are not reached by this tool.
