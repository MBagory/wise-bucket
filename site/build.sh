#!/bin/sh
# Builds site/style.css from site/input.css with the Tailwind CSS standalone
# CLI, downloaded once into target/ at a pinned version and checked against its
# SHA-256. Extra arguments go to the CLI: `site/build.sh --watch`.
set -eu

version=4.3.3
root=$(cd "$(dirname "$0")/.." && pwd)

# Checksums from the release's sha256sums.txt.
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) asset=tailwindcss-linux-x64 sha=dc61b3ac6b8c9ca874c0cc4c57b2409791a64c5540404ca5f5367360babc313a ;;
    Linux-aarch64 | Linux-arm64) asset=tailwindcss-linux-arm64 sha=55fd0b241214eff3de1e8ee4f22796662f2d2e7a49bcfca7477cfd0bac398195 ;;
    Darwin-arm64) asset=tailwindcss-macos-arm64 sha=cdf646702987a743464dff4d9c60fd4480d1c1e73dd819a9a67f1078815dce9d ;;
    Darwin-x86_64) asset=tailwindcss-macos-x64 sha=7922e0953f2110c05976e3bf58f14e643d90427575e766b7d433f5f80cbee7e1 ;;
    *) echo "site/build.sh: no Tailwind CSS CLI for $(uname -sm)" >&2; exit 1 ;;
esac

bin="$root/target/tailwindcss-$version/$asset"
if [ ! -x "$bin" ]; then
    mkdir -p "$(dirname "$bin")"
    curl --proto '=https' --tlsv1.2 -fsSL -o "$bin.part" \
        "https://github.com/tailwindlabs/tailwindcss/releases/download/v$version/$asset"
    actual=$( (sha256sum "$bin.part" 2>/dev/null || shasum -a 256 "$bin.part") | cut -d ' ' -f 1)
    if [ "$actual" != "$sha" ]; then
        rm -f "$bin.part"
        echo "site/build.sh: checksum mismatch for $asset $version" >&2
        exit 1
    fi
    chmod +x "$bin.part"
    mv "$bin.part" "$bin"
fi

exec "$bin" --input "$root/site/input.css" --output "$root/site/style.css" --minify "$@"
