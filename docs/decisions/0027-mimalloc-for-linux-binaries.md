# 0027. Linux binaries allocate with mimalloc

**Date:** 2026-10-15 · **Status:** Accepted

## Context

The release's Linux binaries are static musl builds (`dist-workspace.toml`), so they run on any distribution. In the 0.1.0 dry run, `SELECT count(DISTINCT value % 1000003) FROM generate_series(1, 50000000)` took 11.6 s with the x86_64 musl binary in Docker on a 12-thread Intel Mac. The macOS binary took 1.1 s on the same machine. Limited to 1, 2 and 4 CPUs, the musl binary took 9.8 s, 18.2 s and 24.7 s: more threads made it slower. musl's allocator serializes threads that allocate at the same time, and DataFusion allocates on every partition's thread.

## Decision

On `target_env = "musl"`, `wisebucket` uses mimalloc as its global allocator (`crates/wb-server/src/lib.rs`). macOS builds and glibc builds from source keep the system allocator.

With mimalloc, in runs alternating on the same machine, the query took 3.0–3.6 s in Linux, against 1.9–2.4 s on macOS and 12.6–15.3 s with musl's allocator. On 4 CPUs it took 4.4 s, where musl's allocator reached the 30 s query cap.

## Alternatives

- **glibc binaries:** they have a fast allocator, but set a glibc floor (2.35 when built on Ubuntu 22.04) that excludes older distributions.
- **jemalloc** (`tikv-jemallocator`, which ripgrep uses on musl): the same effect, with a heavier C build.
- **mimalloc everywhere:** it would change allocators that are not slow.

## Consequences

- Linux builds compile one more C library (`libmimalloc-sys`, MIT) with musl-gcc in the release workflow.
- The same query is the check for a future allocator or build change.
