#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SOCK="${SYNAPSE_SOCK:-/tmp/synapse-agent-memory-bench.sock}"
DB="$(mktemp -t synapse-agent-memory.XXXXXX.db)"
LOG="$(mktemp -t synapse-agent-memory.XXXXXX.log)"
RUN_ID="${RUN_ID:-RBK-PUBLIC-$(date +%Y%m%d-%H%M%S)}"
PYTHON="${PYTHON:-python3}"

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]]; then
    kill "$DAEMON_PID" >/dev/null 2>&1 || true
    wait "$DAEMON_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCK" "$DB" "$LOG"
}
trap cleanup EXIT

cd "$ROOT"
cargo build --quiet -p synapsed --bin synapsed

rm -f "$SOCK"
target/debug/synapsed --file "$DB" --sock "$SOCK" --lazy-embed >"$LOG" 2>&1 &
DAEMON_PID=$!

for _ in {1..100}; do
  [[ -S "$SOCK" ]] && break
  sleep 0.05
done

if [[ ! -S "$SOCK" ]]; then
  echo "synapsed did not create socket; log follows:" >&2
  cat "$LOG" >&2
  exit 1
fi

export SYNAPSE_SOCK="$SOCK"
export PYTHONPATH="$ROOT/sdk/python:${PYTHONPATH:-}"

"$PYTHON" bench/recall_bakeoff/run.py \
  --engines sqlite,synapse,synapse_scoped,synapse_mem0,claude_mem \
  --run-id "$RUN_ID"
