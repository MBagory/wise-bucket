<div align="center">

<img src="docs/assets/banner.svg" alt="Wise Bucket: Fast &amp; Frugal MCP server for your time-series data" width="100%">

[![CI](https://github.com/MBagory/wise-bucket/actions/workflows/ci.yml/badge.svg)](https://github.com/MBagory/wise-bucket/actions/workflows/ci.yml)
[![Docs](https://img.shields.io/badge/docs-guide-blue)](docs/guide.md)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rust-1.98%2B-orange.svg)](Cargo.toml)
![Status: alpha](https://img.shields.io/badge/status-alpha-red)

[Why](#why-wise-bucket) · [Getting started](#getting-started) · [Sample data](#try-it-with-sample-data) · [Everyday use](#everyday-use) · [Docs](#documentation)

</div>

**Your AI agent queries every recording on your machine with SQL.**
ROS 2 bags (MCAP or `.db3`), MCAP (ROS 2, protobuf, JSON), PX4 ULog, MAVLink `.tlog`, ArduPilot DataFlash, Vector CAN logs (ASC, BLF) with your DBC files, NMEA 0183 and NMEA 2000, MDF4, CSV, JSON Lines, Parquet and Arrow IPC.

## Why Wise Bucket

| ⚡ Fast | 🪶 Frugal | 🤖 Bring your own agent | 🎯 Faithful |
| --- | --- | --- | --- |
| Each recording is decoded once into Parquet. Every later question is a SQL query, answered in milliseconds across all your recordings. | Your agent's SQL runs on your machine and only the answer enters its context. Full-resolution signals go straight to your viewer. | Claude Code, Codex, Cursor… with their model and your subscription. No other chat app, no extra API key. | Never worse than your agent alone: values checked against the reference decoders, and what isn't decoded is kept raw or counted in the notes. |

### Compared with your agent alone

| | Agent on the raw files | Agent with Wise Bucket |
| --- | --- | --- |
| Each question | writes and runs a parsing script | one SQL query on decoded columns |
| What fills its context | script output, tracebacks, retries | the answer rows only |
| Across recordings | one file at a time | all of them in one query |
| Decoding | whichever library it picks, however it uses it | checked against python-can, pyulog, pymavlink, pynmea2, canboat and asammdf |
| Plots | prints arrays or writes plotting code | full resolution in Rerun, or the original recording in your viewer |

**How it stays faithful:** [`evals/crosscheck.sh`](evals/crosscheck.sh) has an agent investigate the same recordings twice, once on its own from the raw files and once through Wise Bucket. Each difference confirmed against the files becomes a test, with the reference decoder's value ([Contributing](CONTRIBUTING.md)).

### Also

- 📈 **Instant display:** your agent opens the original recording at the right moment in the viewer you already use, through its MCP server ([Rerun, Foxglove…](docs/guide.md#viewers)).
- 📝 **Long-term context:** [tell your agent](docs/guide.md#what-you-know-about-recordings) what the files don't say (the machine, what changed, the conditions, the purpose of a test). Wise Bucket keeps it for every later session.
- 🔍 **[Abnormal and similar recordings](docs/guide.md#abnormal-and-similar-recordings):** your agent finds them by comparing each recording with its peers.
- 📋 **Notes:** each recording lists its errors, warnings, gaps and what wasn't decoded.

### How it works

```mermaid
flowchart TB
    rec[/"Your recordings<br/>ROS 2, MCAP, ULog, CSV…"/] -->|"decoded once"| store
    subgraph wb["Wise Bucket"]
        store[("Parquet files")] --> engine["DataFusion<br/>SQL engine"] --> mcp["MCP server"]
    end
    you(["You"]) -.->|"CLI configuration"| wb
    mcp -->|"SQL, in milliseconds"| agent["Your AI agent<br/>Claude Code, Codex, Cursor…"]
    mcp -->|"query results"| viewer["Viewer<br/>Rerun, Foxglove"]
    agent -->|"displays the data"| viewer
    agent <-->|"questions, answers"| you
```

Written in Rust, on [Apache DataFusion](https://datafusion.apache.org) and [Apache Parquet](https://parquet.apache.org).

> [!WARNING]
> **Early alpha.** Commands and on-disk formats may change. [Issues](https://github.com/MBagory/wise-bucket/issues) welcome.

## Getting started

**1. Install** on macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/MBagory/wise-bucket/releases/latest/download/wisebucket-installer.sh | sh
```

Then open a new terminal. To update, run it again. To build from source instead, see the [guide](docs/guide.md#build-from-source).

**2. Set up** (once): `wisebucket setup` asks which folders hold your recordings.

**3. Connect your agent** (once) with the command `setup` prints. Claude Code:

```sh
claude mcp add --scope user wise-bucket -- ~/.cargo/bin/wisebucket serve
```

Other agents (Codex, Cursor, Cline…): see [Connect your agent](docs/guide.md#connect-your-agent).

**4. Ask**, in a new agent session:

```text
you   › In the demo ULog, what is the accelerometer noise floor at rest?

agent › ⏺ wise-bucket · sql
          SELECT count(*), stddev("accelerometer_m_s2[2]"), avg("accelerometer_m_s2[2]")
          FROM stream('sensor_combined')
        Over 17,070 samples, the vertical acceleration averages −9.62 m/s²
        (gravity) with a standard deviation of 0.16 m/s²: that is the noise
        floor at rest.

you   › Show me the three accelerometer axes in Rerun.

agent › ⏺ wise-bucket · rerun  (entity: sensor_combined)
          SELECT t, "accelerometer_m_s2[0]" AS x, "accelerometer_m_s2[1]" AS y,
                 "accelerometer_m_s2[2]" AS z
          FROM stream('sensor_combined')
        Sent 17,070 rows to the Rerun viewer: sensor_combined/x, y and z,
        on the timeline t.
```

The Rerun viewer must be running (`rerun`). Wise Bucket sends the rows to it directly; they do not pass through your agent.

## Try it with sample data

```sh
wisebucket demo --all     # about 4 MB of public samples, declared as the `demo` folder
wisebucket demo --list    # formats, sizes and licenses
```

Samples are fetched from their upstream projects at pinned commits, SHA-256-verified, and stay under their own licenses. Then ask your agent:

| Sample | Ask |
| --- | --- |
| `ros2-mcap` · `ros2-db3` | *"Is `/topic` published at a steady rate?"* |
| `ulog` | *"When does CPU load peak, and what runs then?"* · *"Why is the barometric altitude flat?"* · *"Which parameters differ from their defaults?"* |
| `tlog` | *"What battery state was reported?"* |
| `dataflash` | *"How far did the attitude lag the demand?"* · *"Which parameters were set?"* |
| `nmea0183` | *"What was the speed over ground, and how many satellites?"* |
| `nmea2000` | *"How did the apparent wind change during the tack?"* |
| `mdf4` | *"What range does each channel cover?"* |
| `can` | *"Which CAN IDs are on the bus, at what rate?"* · *"When did error frames occur?"* |
| `parquet` | *"Which column types does it hold?"* |

## Everyday use

| You want to… | Run |
| --- | --- |
| Add a recording folder | `wisebucket roots add flights /Volumes/lab-nas/px4` |
| Tell what the files don't say | `wisebucket note flights: hardware=drone-2`, or just tell your agent |
| Query the store as your agent does | `wisebucket sql "SELECT status, count(*) FROM recordings GROUP BY status"` |
| Check that everything works | `wisebucket doctor` |

Runs on macOS and Linux; Windows through WSL2.

## Documentation

[User guide](docs/guide.md) · [Troubleshooting and FAQ](docs/troubleshooting.md) · Reference: [tables](docs/reference/tables.md), [CLI](docs/reference/cli.md), [configuration](docs/reference/configuration.md), [errors](docs/reference/errors.md) · [Security and privacy](SECURITY.md) · [Contributing](CONTRIBUTING.md)

## License

[Apache License 2.0](LICENSE). Built on [Apache DataFusion](https://datafusion.apache.org), [Apache Parquet](https://parquet.apache.org), [Apache Arrow](https://arrow.apache.org), the [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk), [mcap](https://github.com/foxglove/mcap), Rerun's [re_ros_msg](https://github.com/rerun-io/rerun), [px4-ulog](https://github.com/PX4/px4-ulog-rs), [rust-mavlink](https://github.com/mavlink/rust-mavlink) and [SQLite](https://sqlite.org). Bundled ROS 2 message definitions come from [ros2/common_interfaces](https://github.com/ros2/common_interfaces) and related repositories (Apache-2.0, BSD-3-Clause). To cite Wise Bucket, use [CITATION.cff](CITATION.cff).
