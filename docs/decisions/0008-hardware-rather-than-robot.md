# 0008. "Hardware" rather than "robot"

**Date:** 2026-10-05 · **Status:** Root option superseded by [0025](0025-context-from-the-engineer.md); the vocabulary is kept

## Context

Wise Bucket was described in robot terms (`roots add --robot`, "teach Wise Bucket your robot"). Its users are R&D teams working with time-series data from any hardware: robots, but also drones, vehicles and test benches.

## Decision

The vocabulary is **hardware**: the data-root option is `hardware` (`roots add --hardware`), shown in the `recordings` table. Robots remain the main example.

## Alternatives

- **Keeping "robot":** narrower than the audience.
- **An alias for the old `robot` key:** not needed before the first release; an old key fails with `config_invalid`, and the changelog says to rename it.

## Consequences

- Configurations written with `robot = "…"` must be edited once.
