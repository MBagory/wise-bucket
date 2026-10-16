# 0016. A format ships only when an external Rust crate decodes it

**Date:** 2026-10-11 · **Status:** Accepted

## Context

[0002](0002-native-rust-log-readers.md) chose native Rust readers, using maintained crates where they exist. The demo samples covered formats that no crate decodes: ROS 1 messages (in bags and in MCAP), and CAN frames decoded with a DBC file (`can-dbc` parses DBC files but decodes no frames). Writing those decoders ourselves means owning a wire format's edge cases, and its bugs give wrong numbers rather than errors.

## Decision

Wise Bucket reads a format only when an **external Rust crate decodes it**; our code only maps the crate's output to streams. The demo offers only such formats. The others are listed as possible features ([0014](0014-possible-features-instead-of-milestones.md)) until a crate exists.

Crates added for this rule: `mavlink` (MAVLink telemetry logs, `ardupilotmega` dialect), `blf_asc` (Vector CAN logs, ASC and BLF), `falcon_mdf` (CAN frames decoded with a DBC file, [0029](0029-can-signals-from-confirmed-dbc.md)). Messages are turned into columns through `serde` and Arrow's JSON decoder, so a message's arrays stay arrays.

## Alternatives

- **Write the missing decoders** (ROS 1 to CDR, DBC bit extraction): a few hundred lines each, but every edge case is ours to find.
- **Python readers:** set aside by [0002](0002-native-rust-log-readers.md).

## Consequences

- Fewer formats, each backed by a crate that others test too.
- A young crate can still be wrong: each format keeps tests with exact expected values on its demo file.
- ROS 1 bags and ROS 1-encoded MCAP wait for a crate.
