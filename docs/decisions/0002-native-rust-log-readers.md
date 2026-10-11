# 0002. Native Rust log readers; no Python readers, no Bagel

**Date:** 2026-09-28 · **Status:** Accepted

## Context

Reading recordings (ROS 2 bags, MCAP, ULog…) is at the core of Wise Bucket. Python ecosystems have readers for most formats, and [Bagel](https://github.com/Extelligence-ai/bagel) offers an MCP server that converts recordings for an LLM.

## Decision

Recordings are read by **native Rust readers** inside Wise Bucket, using maintained crates where they exist (`mcap`, `re_ros_msg`, `px4-ulog`, `rusqlite`, DataFusion's CSV, JSON, Parquet and Arrow readers).

## Alternatives

- **Python readers** (rosbags, pyulog, asammdf…): mature, but they bring a Python environment to install and maintain on every machine.
- **Bagel:** converts the topics a question needs on each question, into Arrow files queried by DuckDB. It is the closest project, but it keeps nothing between questions and cannot query across many recordings at once ([0012](0012-trade-storage-for-query-speed.md)).

## Consequences

- One binary, no runtime to install.
- Each new format is Rust work; the order in which formats arrive follows demand ([0014](0014-possible-features-instead-of-milestones.md)).
- Decoder correctness needs tests with exact expected values on fixtures.
