#!/usr/bin/env bash
# run.sh — orchestrator: start qdrant, run bench.py per engine, aggregate
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="$HOME/.venvs/synapse-bench"
QDRANT_BIN="$HOME/.local/bin/qdrant"
DRY_RUN="${DRY_RUN:-0}"
N_DOCS="${N_DOCS:-100000}"
LOG="$DIR/run.log"

source "$VENV/bin/activate"

# Prevent thermal throttle
pmset noidle &
NOIDLE_PID=$!
cleanup() {
  kill $NOIDLE_PID 2>/dev/null || true
  kill $QDRANT_PID 2>/dev/null || true
}
trap cleanup EXIT

# Start Qdrant if binary exists
QDRANT_PID=0
if [ -x "$QDRANT_BIN" ]; then
  echo "[run] Starting Qdrant..."
  QDRANT_DATA=$(mktemp -d)
  "$QDRANT_BIN" --storage-path "$QDRANT_DATA" --port 6333 > "$DIR/qdrant.log" 2>&1 &
  QDRANT_PID=$!
  sleep 3
  if ! kill -0 $QDRANT_PID 2>/dev/null; then
    echo "[warn] Qdrant failed to start, will skip qdrant engine"
    QDRANT_PID=0
  else
    echo "[run] Qdrant PID=$QDRANT_PID"
  fi
else
  echo "[warn] Qdrant binary not found at $QDRANT_BIN, skipping"
fi

# Generate dataset if needed
if [ ! -f "$DIR/dataset.parquet" ]; then
  echo "[run] Generating dataset..."
  bash "$DIR/setup.sh" "$N_DOCS"
fi

# Select engines
ALL_ENGINES="sqlite-vec,duckdb,lancedb,chromadb,synapse"
if [ $QDRANT_PID -ne 0 ]; then
  ALL_ENGINES="$ALL_ENGINES,qdrant"
fi

DRY_FLAG=""
if [ "$DRY_RUN" = "1" ]; then
  DRY_FLAG="--dry-run"
  echo "[run] DRY RUN MODE (1k docs)"
fi

echo "[run] Engines: $ALL_ENGINES"
echo "[run] Starting benchmark at $(date)"

# Run each engine as separate process to avoid segfault from mixed .so loading
IFS=',' read -ra ENGINE_LIST <<< "$ALL_ENGINES"
for engine in "${ENGINE_LIST[@]}"; do
  echo "[run] --- $engine ---"
  python3 "$DIR/bench.py" \
    --engine "$engine" \
    --n-docs "$N_DOCS" \
    $DRY_FLAG 2>&1 | tee -a "$LOG"
done

echo "[run] Benchmark complete at $(date)"

# Aggregate and generate report
python3 "$DIR/report.py"

echo "[run] Report written to $DIR/RESULTS.md"
