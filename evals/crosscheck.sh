#!/usr/bin/env bash
# Cross-check: on recordings, Claude compares what it finds through Wise Bucket with
# what it finds on its own from the raw files. Writes one report per sample format or
# per folder to evals/reports/, to paste into a dev session.
#
# Usage: evals/crosscheck.sh [format…]   public samples (no format: choose interactively)
#        evals/crosscheck.sh <folder>…   your own recordings
# Environment: CC_MODEL (default sonnet), CC_BUDGET in USD per report (default 5).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
model=${CC_MODEL:-sonnet}
budget=${CC_BUDGET:-5}
reports=$repo/evals/reports
wb=$repo/target/release/wisebucket

# The code under development, in a store of its own.
cargo build --release -q -p wisebucket --manifest-path "$repo/Cargo.toml"
mkdir -p "$reports"

# One report per unit: name|folder|root|what the folder holds.
units=()
if [ $# -gt 0 ] && [ -d "$1" ]; then
  work=$repo/target/crosscheck/folders
  rm -rf "$work"
  mkdir -p "$work"
  export WB_CONFIG=$work/config.toml WB_STATE_DIR=$work/state
  for d in "$@"; do
    d=$(cd "$d" && pwd)
    # Root names: lowercase letters, digits, '-' and '_', at most 32 characters.
    name=$(printf %s "$(basename "$d")" | tr 'A-Z' 'a-z' | tr -c 'a-z0-9_-' '-' | sed 's/^[^a-z0-9]*//' | cut -c1-32)
    "$wb" roots add "$name" "$d" >/dev/null
    units+=("$name|$d|$name|recordings")
  done
else
  work=$repo/target/crosscheck
  mkdir -p "$work"
  export WB_CONFIG=$work/config.toml WB_STATE_DIR=$work/state
  demo=$("$wb" --json demo "$@")
  dir=$(jq -r .dir <<<"$demo")
  for f in $(jq -r '.formats[]' <<<"$demo"); do
    units+=("$f|$dir/$f|demo|public sample recordings of the format \`$f\`")
  done
fi
[ ${#units[@]} -gt 0 ] || exit 0

# Every recording extracted from scratch before the agents start: a server whose worker
# runs while its stdin stays open.
rm -rf "$WB_STATE_DIR/store"
fifo=$work/stdin
rm -f "$fifo" && mkfifo "$fifo"
"$wb" serve <"$fifo" >/dev/null 2>&1 &
server=$!
exec 3>"$fifo"
printf '%s\n' \
  '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"crosscheck","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' >&3
left=""
for _ in $(seq 300); do
  left=$("$wb" --json sql "SELECT count(*) AS n, count(*) FILTER (WHERE status IN ('pending','extracting','paused')) AS left FROM recordings" |
    jq -r '.rows[0] | if .[0] > 0 then .[1] else "none found" end')
  [ "$left" = 0 ] && break
  sleep 1
done
exec 3>&-
wait "$server" || true
rm -f "$fifo"
if [ "$left" != 0 ]; then
  echo "extraction not finished after 300 s ($left left):" >&2
  "$wb" sql "SELECT recording, status FROM recordings WHERE status <> 'done'" >&2
  exit 1
fi
"$wb" sql "SELECT status, count(*) AS n FROM recordings GROUP BY status"

read -rp "Cross-check ${#units[@]} report(s) with $model, up to \$$budget each (\$$((${#units[@]} * budget)) in all)? [y/N] " ok
[ "$ok" = y ] || exit 0

cat >"$work/mcp.json" <<JSON
{"mcpServers": {"wise-bucket": {"command": "$wb", "args": ["serve"],
  "env": {"WB_CONFIG": "$WB_CONFIG", "WB_STATE_DIR": "$WB_STATE_DIR"}}}}
JSON

for u in "${units[@]}"; do
  IFS='|' read -r name dir root what <<<"$u"
  report=$reports/$(date +%Y-%m-%d-%H%M)-$name.md
  echo "→ $name"
  prompt=$(sed -e "s|{{WHAT}}|$what|g" -e "s|{{DIR}}|$dir|g" -e "s|{{ROOT}}|$root|g" \
    "$repo/evals/crosscheck-prompt.md")
  # No repo context, hooks, memory or other MCP servers: a fresh agent on the recordings.
  out=$(cd "$dir" && claude -p "$prompt" --model "$model" --max-budget-usd "$budget" \
    --setting-sources "" --strict-mcp-config --mcp-config "$work/mcp.json" \
    --allowedTools "mcp__wise-bucket__*" Bash Read Glob Grep \
    --output-format json) || true
  {
    echo "# Cross-check: $name"
    echo
    jq -r --arg m "$model" --arg v "$(git -C "$repo" describe --always --dirty)" '
      "Model \($m) · commit \($v) · $\(.total_cost_usd // 0 | . * 100 | round / 100) · \(.num_turns // "?") turns · \(.subtype // "error")"
      + (if .subtype == "success" then "" else " (partial)" end)' <<<"$out" 2>/dev/null || echo "Claude failed"
    echo
    jq -r '.result // empty' <<<"$out" 2>/dev/null
  } >"$report"
  echo "  $report"
done
