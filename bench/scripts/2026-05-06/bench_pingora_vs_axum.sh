#!/usr/bin/env bash
# Bench: axum (synapsed :9477) vs pingora (synapse-edge :9478)
# Requires: oha (preferred) or curl+hyperfine

set -euo pipefail

AXUM_URL="http://127.0.0.1:9477/health"
PINGORA_URL="http://127.0.0.1:9478/health"
RESULTS_DIR="$(dirname "$0")/../../results/2026-05-06"
OUT="$RESULTS_DIR/pingora-vs-axum.md"
mkdir -p "$RESULTS_DIR"

check_up() {
  curl -sf --max-time 1 "$1" -o /dev/null
}

if ! check_up "$AXUM_URL"; then
  echo "ERROR: axum/synapsed not reachable at $AXUM_URL — start synapsed first" >&2
  exit 1
fi
if ! check_up "$PINGORA_URL"; then
  echo "ERROR: synapse-edge not reachable at $PINGORA_URL — start synapse-edge first" >&2
  exit 1
fi

run_bench() {
  local name="$1"
  local url="$2"
  if command -v oha &>/dev/null; then
    oha -c 256 -q 5000 -z 30s --no-tui --json "$url" 2>/dev/null
  else
    echo "oha not found — install with: cargo install oha" >&2
    hyperfine --runs 3 "curl -sf $url -o /dev/null" --export-json /tmp/hf_${name}.json 2>&1
    cat /tmp/hf_${name}.json
  fi
}

echo "# Pingora vs Axum Benchmark — $(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$OUT"
echo "" >> "$OUT"
echo "## Setup" >> "$OUT"
echo "- axum (synapsed): \`$AXUM_URL\`" >> "$OUT"
echo "- pingora (synapse-edge): \`$PINGORA_URL\`" >> "$OUT"
echo "- oha: \`oha -c 256 -q 5000 -z 30s\`" >> "$OUT"
echo "" >> "$OUT"

echo "## Axum (synapsed :9477)" >> "$OUT"
echo '```' >> "$OUT"
run_bench axum "$AXUM_URL" >> "$OUT"
echo '```' >> "$OUT"
echo "" >> "$OUT"

echo "## Pingora (synapse-edge :9478)" >> "$OUT"
echo '```' >> "$OUT"
run_bench pingora "$PINGORA_URL" >> "$OUT"
echo '```' >> "$OUT"

echo "Results written to: $OUT"
