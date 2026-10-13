# 0022. Unreadable formats are listed

**Date:** 2026-10-13 · **Status:** Accepted

## Context

Wise Bucket listed ROS 1 bags as `unsupported`, and ignored every other file it could not read. The `demo` command downloads ArduPilot, CAN and MDF4 samples: the two CAN logs sat in the `demo` folder but did not appear in `recordings`, so an agent asked about the folder could not tell they existed.

## Decision

- ArduPilot DataFlash logs, CAN ASC and BLF logs, and MDF4 files are recognized and listed with the status `unsupported`, like ROS 1 bags. Their `kind` is `ardupilot_bin`, `can_asc`, `can_blf` or `mdf4`.
- `.blf` and `.mf4` are recognized by their extension.
- `.bin` and `.asc` are generic extensions, so their first bytes are checked too: `A3 95 80` (the first FMT message of a DataFlash log) and `date ` (the first line of a Vector ASC log).
- CAN decoding databases (`.dbc`) describe signals and are not recordings.

## Alternatives

- **Extensions alone:** firmware images (`.bin`) and PGP signatures (`.asc`) would be listed as recordings.
- **Leaving them out:** the agent cannot tell the engineer that these files exist and are not readable yet.

## Consequences

- An agent sees every recording of a folder, readable or not, and says which ones it cannot read.
- Discovery opens each `.bin` and `.asc` file to read its first bytes.
- Reading one of these formats later turns its `unsupported` recordings into extracted ones, with no change in discovery.
