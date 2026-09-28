# Spike: provisioning pgvector for the managed PostgreSQL (M0)

**Date:** 2026-09-27 · **Outcome:** ✅ build from source works on macOS; adopted as the M0 default.

## Question

The managed mode needs PostgreSQL + pgvector ≥ 0.8 on Linux and macOS without asking users to install PostgreSQL. pgvector upstream publishes source only; third-party binary builds (e.g. portalcorp) cover PostgreSQL 16 only.

## What was tested

1. Downloaded `postgresql-17.11.0-x86_64-apple-darwin.tar.gz` from [theseus-rs/postgresql-binaries](https://github.com/theseus-rs/postgresql-binaries) (the builds used by the `postgresql_embedded` crate). SHA-256 matches the published checksum.
2. The archive contains `bin/pg_config`, `include/server` and `lib/pgxs/src/makefiles/pgxs.mk`; `pg_config` reports relocatable paths.
3. Built pgvector **v0.8.6** (SHA-256 `10bf9938…7f8f`) with `make PG_CONFIG=<managed>/bin/pg_config OPTFLAGS=""`.
   - **Problem:** PGXS records the SDK path of the build machine (`/Applications/Xcode_16.4.app/…/MacOSX15.5.sdk`), so `stdio.h` is not found.
   - **Fix:** pass `PG_SYSROOT=$(xcrun --show-sdk-path)`.
   - `OPTFLAGS=""` avoids `-march=native`, so the build is portable across CPUs.
4. Build time: about 14 s on an Intel i7-8850H. `make install` puts `vector.dylib` and the SQL files into the managed installation.
5. `initdb`, then start with a Unix socket only, `CREATE EXTENSION vector` → version 0.8.6, and `'[1,2,3]'::vector <=> '[1,2,4]'::vector` returns the expected cosine distance.

## Decision

- **Default:** build pgvector from its pinned, checksum-verified source against the exact PostgreSQL binaries we download (`Managed::install_pgvector`). This needs `make` and a C compiler (`toolchain_missing` explains how to install them).
- **Not used:** the `postgresql_embedded` crate. We need control over the socket-only configuration, the pgvector build into the same installation, and file locking between processes. Downloading the same theseus archives directly is simple, and it keeps the dependency tree small.
- **Fallback (later, if users lack a toolchain):** a CI workflow producing pgvector binaries per OS/arch against the same PostgreSQL archives, verified by checksum at setup.

## Follow-ups

- Verify on Linux x86_64/arm64 and macOS arm64 in CI (the workflow runs the full M0 suite on ubuntu-latest and macos-14).
- Windows native: needs an MSVC build (planned with native Windows support).
