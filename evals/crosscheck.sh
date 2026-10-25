#!/usr/bin/env bash
# Cross-check: on sample recordings of each chosen format, Claude compares what it
# finds through Wise Bucket with what it finds on its own from the raw files.
# Writes one report per format to evals/reports/, to paste into a dev session.
#
# Usage: evals/crosscheck.sh [format…]   (no format: choose interactively)
# Environment: CC_MODEL (default sonnet), CC_BUDGET in USD per format (default 5).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
model=${CC_MODEL:-sonnet}
budget=${CC_BUDGET:-5}
work=$repo/target/crosscheck
reports=$repo/evals/reports
wb=$repo/target/release/wisebucket

# The code under development, in a store of its own.
cargo build --release -q -p wisebucket --manifest-path "$repo/Cargo.toml"
export WB_CONFIG=$work/config.toml WB_STATE_DIR=$work/state
mkdir -p "$work" "$reports"

demo=$("$wb" --json demo "$@")
dir=$(jq -r .dir <<<"$demo")
formats=$(jq -r '.formats[]' <<<"$demo")
[ -n "$formats" ] || exit 0

n=$(wc -w <<<"$formats" | tr -d ' ')
read -rp "Cross-check $(echo $formats) with $model, up to \$$budget per format (\$$((n * budget)) in all)? [y/N] " ok
[ "$ok" = y ] || exit 0

cat >"$work/mcp.json" <<JSON
{"mcpServers": {"wise-bucket": {"command": "$wb", "args": ["serve"],
  "env": {"WB_CONFIG": "$WB_CONFIG", "WB_STATE_DIR": "$WB_STATE_DIR"}}}}
JSON

for f in $formats; do
  report=$reports/$(date +%Y-%m-%d-%H%M)-$f.md
  echo "→ $f"
  prompt=$(sed -e "s|{{FORMAT}}|$f|g" -e "s|{{DIR}}|$dir/$f|g" "$repo/evals/crosscheck-prompt.md")
  # No repo context, hooks, memory or other MCP servers: a fresh agent on the samples.
  out=$(cd "$dir/$f" && claude -p "$prompt" --model "$model" --max-budget-usd "$budget" \
    --setting-sources "" --strict-mcp-config --mcp-config "$work/mcp.json" \
    --allowedTools "mcp__wise-bucket__*" Bash Read Glob Grep \
    --output-format json) || true
  {
    echo "# Cross-check: $f"
    echo
    jq -r --arg m "$model" --arg v "$(git -C "$repo" describe --always --dirty)" '
      "Model \($m) · commit \($v) · $\(.total_cost_usd // 0 | . * 100 | round / 100) · \(.num_turns // "?") turns · \(.subtype // "error")"
      + (if .subtype == "success" then "" else " (partial)" end)' <<<"$out" 2>/dev/null || echo "Claude failed"
    echo
    jq -r '.result // empty' <<<"$out" 2>/dev/null
  } >"$report"
  echo "  $report"
done
