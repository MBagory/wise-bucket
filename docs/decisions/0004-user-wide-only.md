# 0004. User-wide only; recordings organized by data root

**Date:** 2026-10-02 · **Status:** Accepted

## Context

M0 first had two configuration layers: user-wide, and per code repository (`.wisebucket/config.toml`, an `init` command). Recordings are rarely organized by code repository: they live in folders on disks and NAS mounts, shared across projects.

## Decision

Configuration is **user-wide only**. The engineer declares named **data roots** (folders Wise Bucket may read) with `wisebucket roots add`; recordings are referenced as `root:relative/path`. The agent is registered once per machine. No MCP tool can add a root.

## Alternatives

- **Per-repository configuration:** removed with `init`; it tied recordings to code repositories they don't belong to.

## Consequences

- One place to configure, one registration per machine.
- Moving a mount point only changes a root's path.
- Sharing configuration within a team is not covered (see the team features in [0014](0014-possible-features-instead-of-milestones.md)).
