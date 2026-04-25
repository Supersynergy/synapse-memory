#!/usr/bin/env bash
# run.sh — orchestrator: start qdrant, run bench.py per engine, aggregate
set -uo pipefail   # NOTE: no -e; each engine runs in isolation, segfaults don't abort all

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="$HOME/.venvs/synapse-bench"
QDRANT_BIN="$HOME/.local/bin/qdrant"
DRY_RUN="${DRY_RUN:-0}"
N_DOCS="${N_DOCS:-100000}"
PHASES="${PHASES:-base}"
LOG="$DIR/run.log"

source "$VENV/bin/activate"

# Prevent thermal throttle
pmset noidle &
NOIDLE_PID=$!
QDRANT_PID=0
QDRANT_DATA=""

cleanup() {
  kill $NOIDLE_PID 2>/dev/null || true
  if [ $QDRANT_PID -ne 0 ]; then
    kill $QDRANT_PID 2>/dev/null || true
  fi
  if [ -n "$QDRANT_DATA" ] && [ -d "$QDRANT_DATA" ]; then
    rm -rf "$QDRANT_DATA"
  fi
}
trap cleanup EXIT

# Kill any lingering processes on qdrant ports
lsof -ti:6333 | xargs kill -9 2>/dev/null || true
lsof -ti:6334 | xargs kill -9 2>/dev/null || true
sleep 2

# Start Qdrant if binary exists
if [ -x "$QDRANT_BIN" ]; then
  echo "[run] Starting Qdrant..."
  QDRANT_DATA=$(mktemp -d /tmp/qdrant-bench-XXXXXX)
  "$QDRANT_BIN" --storage-path "$QDRANT_DATA" --port 6333 > "$DIR/qdrant.log" 2>&1 &
  QDRANT_PID=$!
  # Health-check loop: up to 10s
  QDRANT_UP=0
  for i in $(seq 1 10); do
    sleep 1
    if curl -sf http://localhost:6333/healthz >/dev/null 2>&1; then
      QDRANT_UP=1
      break
    fi
    if ! kill -0 $QDRANT_PID 2>/dev/null; then
      break
    fi
  done
  if [ $QDRANT_UP -eq 1 ]; then
    echo "[run] Qdrant PID=$QDRANT_PID ready"
  else
    echo "[warn] Qdrant failed health-check, will skip qdrant engine"
    kill $QDRANT_PID 2>/dev/null || true
    QDRANT_PID=0
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

PHASES_FLAG="--phases=$PHASES"

echo "[run] Engines: $ALL_ENGINES"
echo "[run] Phases: $PHASES"
echo "[run] Starting benchmark at $(date)"

# Run each engine as separate process to avoid segfault from mixed .so loading
# No -e: a segfault in one engine should not stop the rest
IFS=',' read -ra ENGINE_LIST <<< "$ALL_ENGINES"
for engine in "${ENGINE_LIST[@]}"; do
  echo "[run] --- $engine ---"
  # Run in subshell; capture exit code without aborting whole script
  set +e
  python3 "$DIR/bench.py" \
    --engine "$engine" \
    --n-docs "$N_DOCS" \
    $DRY_FLAG \
    $PHASES_FLAG 2>&1 | tee -a "$LOG"
  EXIT_CODE=$?
  set -e
  if [ $EXIT_CODE -ne 0 ]; then
    echo "[warn] engine=$engine exited with code=$EXIT_CODE (segfault=139), continuing..."
  fi
done

echo "[run] Benchmark complete at $(date)"

# Aggregate and generate report
python3 "$DIR/report.py" --partial

echo "[run] Report written to $DIR/RESULTS.md"
