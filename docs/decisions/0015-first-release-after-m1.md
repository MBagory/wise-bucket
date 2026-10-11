# 0015. First release after M1 is tested

**Date:** 2026-10-10 · **Status:** Accepted (supersedes [0007](0007-no-release-before-the-mvp.md))

## Context

[0007](0007-no-release-before-the-mvp.md) waited for an MVP at milestone M4, which no longer exists ([0014](0014-possible-features-instead-of-milestones.md)). M1 (an automatic columnar store of every recording, queried with SQL by the agent) is useful on its own, and without PostgreSQL the binary is self-contained.

## Decision

The **first release** (prebuilt binaries and an installer, built by `dist`) comes **after the maintainer has tested M1**, as a separate step. Until then, install from source.

## Alternatives

- **Waiting for more features:** delays feedback on what already works.

## Consequences

- The release step must give `dist` the build dependency `protoc`, and switch the README from install-from-source to the installer.
