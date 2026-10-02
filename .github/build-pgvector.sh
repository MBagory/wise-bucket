#!/bin/sh
# Builds pgvector against Wise Bucket's pinned PostgreSQL binaries and packages
# the files `wisebucket setup` unpacks into the managed install.
#
# Usage: .github/build-pgvector.sh <target-triple> <out-dir>   (run from the repo root)
#
# Linux: run it in a rockylinux:9 container. Its glibc (2.34) is the floor the
# PostgreSQL binaries already require, so vector.so never raises it.
set -eu

target=$1
out=$(mkdir -p "$2" && cd "$2" && pwd)

# Versions and the PostgreSQL checksum come from managed.rs, the single source of truth.
src=crates/wb-core/src/db/managed.rs
pg_version=$(sed -n 's/^pub const PG_VERSION: &str = "\(.*\)";/\1/p' "$src")
pgvector_version=$(sed -n 's/^pub const PGVECTOR_VERSION: &str = "\(.*\)";/\1/p' "$src")
pg_sha=$(grep -A1 "\"$target\"," "$src" | sed -n 's/.*"\([0-9a-f]\{64\}\)".*/\1/p' | head -1)
# SHA-256 of https://github.com/pgvector/pgvector/archive/refs/tags/v0.8.6.tar.gz
pgvector_sha=10bf9938906e5d643bbc4a7eea104b6f57ba4898e5b76b20e60484ea1d5a7f8f

fetch() { # url sha256 file
    curl -fsSL -o "$3" "$1"
    got=$( (sha256sum "$3" 2>/dev/null || shasum -a 256 "$3") | cut -d' ' -f1)
    [ "$got" = "$2" ] || { echo "checksum mismatch for $1: $got" >&2; exit 1; }
}

work=$(mktemp -d)  # no spaces: PGXS (GNU make) cannot handle them
cd "$work"
fetch "https://github.com/theseus-rs/postgresql-binaries/releases/download/$pg_version/postgresql-$pg_version-$target.tar.gz" "$pg_sha" pg.tar.gz
fetch "https://github.com/pgvector/pgvector/archive/refs/tags/v$pgvector_version.tar.gz" "$pgvector_sha" pgvector.tar.gz
tar xzf pg.tar.gz
tar xzf pgvector.tar.gz
pg=$work/postgresql-$pg_version-$target

cd "pgvector-$pgvector_version"
# OPTFLAGS="": no -march=native, so the library runs on any CPU of the architecture.
set -- PG_CONFIG="$pg/bin/pg_config" OPTFLAGS=""
if [ "$(uname)" = Darwin ]; then
    # PGXS records the SDK path of the machine that built PostgreSQL; use ours.
    set -- "$@" PG_SYSROOT="$(xcrun --show-sdk-path)"
    export MACOSX_DEPLOYMENT_TARGET=11.0
fi
make -j4 "$@"
make install "$@"

cd "$pg"
name=pgvector-$pgvector_version-pg$pg_version-$target.tar.gz
tar czf "$out/$name" lib/vector.* share/extension/vector*
tar tzf "$out/$name"
