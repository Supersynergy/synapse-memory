#!/usr/bin/env bash
# benchmark.sh — synapse-mcp coding-agent working-memory benchmark
# Usage: ./benchmark.sh [socket_path]
set -euo pipefail

SOCK="${1:-/tmp/synapse.sock}"
MCP="./target/debug/synapse-mcp -s $SOCK"
N_INGEST=1000
N_SEARCH=100

# ── helpers ──────────────────────────────────────────────────────────────────
mcp_call() {
  local method="$1" params="$2"
  printf '{"jsonrpc":"2.0","id":1,"method":"%s","params":%s}\n' "$method" "$params" \
    | timeout 5 $MCP 2>/dev/null
}

tool_call() {
  local name="$1" args="$2"
  mcp_call "tools/call" "{\"name\":\"$name\",\"arguments\":$args}"
}

# ── check synapsed is up ──────────────────────────────────────────────────────
if [[ ! -S "$SOCK" ]]; then
  echo "ERROR: synapsed not running (no socket at $SOCK)"
  echo "  Start with: synapsed --sock $SOCK"
  exit 1
fi

echo "=== synapse-mcp coding-agent benchmark ==="
echo "Socket : $SOCK"
echo "Binary : $(./target/debug/synapse-mcp --version 2>/dev/null || echo dev)"
echo ""

# ── 1. ingest 1000 fake code-context docs ────────────────────────────────────
echo "--- Ingesting $N_INGEST code-context memories ---"
T0=$(date +%s%3N)
for i in $(seq 1 $N_INGEST); do
  TEXT="fn compute_$i(input: &str) -> String { // context: module auth, file src/auth.rs:$i }"
  tool_call "memory_save" "{\"text\":\"$TEXT\",\"tags\":[\"rust\",\"auth\"]}" > /dev/null
done
T1=$(date +%s%3N)
INGEST_MS=$(( T1 - T0 ))
echo "  Total  : ${INGEST_MS}ms"
echo "  Per doc: $(echo "scale=2; $INGEST_MS / $N_INGEST" | bc)ms"
echo ""

# ── 2. 100 sequential memory_search calls ────────────────────────────────────
echo "--- $N_SEARCH sequential memory_search calls ---"
QUERIES=("compute" "auth" "input str" "String" "module" "context" "file src" "fn " "return" "token")
TIMES=()
for i in $(seq 1 $N_SEARCH); do
  Q="${QUERIES[$(( (i - 1) % ${#QUERIES[@]} ))]}"
  T_START=$(date +%s%3N)
  tool_call "memory_search" "{\"query\":\"$Q\",\"k\":10}" > /dev/null
  T_END=$(date +%s%3N)
  TIMES+=( $(( T_END - T_START )) )
done

# compute p50/p95/p99
IFS=$'\n' SORTED=($(sort -n <(printf '%s\n' "${TIMES[@]}")))
TOTAL_SEARCH=0
for t in "${TIMES[@]}"; do TOTAL_SEARCH=$(( TOTAL_SEARCH + t )); done
P50_IDX=$(( N_SEARCH * 50 / 100 - 1 ))
P95_IDX=$(( N_SEARCH * 95 / 100 - 1 ))
P99_IDX=$(( N_SEARCH * 99 / 100 - 1 ))
echo "  p50 : ${SORTED[$P50_IDX]}ms"
echo "  p95 : ${SORTED[$P95_IDX]}ms"
echo "  p99 : ${SORTED[$P99_IDX]}ms"
echo "  avg : $(echo "scale=1; $TOTAL_SEARCH / $N_SEARCH" | bc)ms"
echo "  Calls/min at p95: $(echo "scale=0; 60000 / ${SORTED[$P95_IDX]}" | bc)"
echo ""

# ── 3. memory_recent ─────────────────────────────────────────────────────────
echo "--- memory_recent(20) ---"
T_S=$(date +%s%3N)
tool_call "memory_recent" "{\"n\":20}" > /dev/null
T_E=$(date +%s%3N)
echo "  ${T_E - T_S}ms"
echo ""

echo "=== done ==="
