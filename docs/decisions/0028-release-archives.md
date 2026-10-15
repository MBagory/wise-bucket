# 0028. Release archives: `.tar.gz` and one binary

**Date:** 2026-10-15 · **Status:** Accepted

## Context

The 0.1.0 archives were `.tar.xz`, dist's default. GNU tar unpacks `.xz` by calling the separate `xz` program, which minimal Linux images lack. In plain `ubuntu:24.04` and `debian:12`, the installer stopped at `tar xf` with `Child returned status 2`. These images have `gzip` but neither `xz`, `zstd` nor `unzip`. macOS's tar reads every format itself.

Each archive also held two full binaries: `wbk` is a second `[[bin]]` with the same `main`, 186 MB unstripped, like `wisebucket`.

Measured on the Linux x86_64 build:

| Archive | Download | Without `xz` | Installed |
| --- | --- | --- | --- |
| Two binaries, `.tar.xz` (0.1.0) | 54 MB | fails | 372 MB |
| Two binaries, `.tar.gz` | 113 MB | works | 372 MB |
| One binary, `.tar.xz` | 29 MB | fails | 186 MB |
| One binary, `.tar.gz` | 56 MB | works | 186 MB |
| One stripped binary, `.tar.gz` | 48 MB | works | 143 MB |

## Decision

- Release archives are `.tar.gz` (`unix-archive` in `dist-workspace.toml`).
- Archives hold `wisebucket` only (`[dist.binaries]`). The installer links `wbk` to it (`[dist.bin-aliases]`, `ln -sf`). `completions` names itself from `argv[0]`, so `wbk completions` still completes `wbk`.
- Builds from source keep both binaries.
- The binaries keep their symbols.

## Alternatives

- **Documenting `xz-utils`:** the installer would still fail for anyone who skips the docs, with an error that does not name `xz`.
- **One binary in `.tar.xz`:** the smallest download (29 MB), but it still needs `xz`.
- **`.zip` or `.tar.zst`:** `unzip` and `zstd` are missing from minimal images too.
- **Stripping symbols:** 8 MB less to download, but panic backtraces lose their function names.

## Consequences

- The download stays the size of 0.1.0's, and the installation takes half the space.
- Users who unpack an archive by hand get `wisebucket` only, with no `wbk`.
