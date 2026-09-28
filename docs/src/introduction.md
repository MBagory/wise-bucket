# Wise Bucket

**Evidence-backed, iterative robot investigations for your AI coding agent.**

When a robot misbehaves, a docking controller that oscillates or a lidar that drops scans when the arm moves, the answer rarely comes from a single look at a single log. You form hypotheses, run a physical test, record new data, measure, and repeat. Wise Bucket keeps that loop **grounded in your own recordings** and **remembered across conversations**.

Wise Bucket is a local [MCP](https://modelcontextprotocol.io) server. Your existing AI harness (Claude Code, Cline, Kilo Code, …) talks to it; Wise Bucket brings:

- **Knowledge of your robot world:** robots, sensors, buses, protocols, remembered once, asked about when unknown.
- **Native readers for robotics logs:** ROS 2 bags first, then PX4, ArduPilot, CAN and more.
- **Metrics with provenance:** every number comes from a traceable computation over a specific recording.
- **Context, metrics and interpretations attached to each log,** searchable across rounds and investigations.

## Two ways to work

1. **Start with a question.** *"Why does the lidar drop scans when the arm moves fast?"* Wise Bucket resolves what you are talking about, asks about what it does not know, helps structure testable hypotheses and tells you exactly what to record next.
2. **Start with data.** Drop in new recordings; Wise Bucket analyzes them, flags anomalies and links them to your open questions. *(Later milestone.)*

## Status

Wise Bucket is **alpha** and built milestone by milestone. This documentation describes **milestone M0 (foundation)**: installation, the local database, data roots and the connection with your harness. See the [roadmap in the README](https://github.com/MBagory/wise-bucket#roadmap).

## Where to go next

- New here? Read [Context](philosophy/context.md) and [Principles](philosophy/principles.md).
- Ready to try? Follow [Install](get-started/install.md), then [Configure](get-started/configure.md) and [Connect your harness](get-started/connect-your-harness.md).
- Something wrong? Run `wise-bucket-server doctor`, then see [Troubleshooting](troubleshooting.md).
