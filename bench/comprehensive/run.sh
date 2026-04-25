#!/usr/bin/env bash
# run.sh — orchestrator: start qdrant, run bench.py per engine, aggregate
set -uo pipefail   # NOTE: no -e; each engine runs in isolation, segfaults don't abort all

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="$HOME/.venvs/synapse-bench"
QDRANT_BIN="$HOME/.local/bin/qdrant"
DRY_RUN="${DRY_RUN:-0}"
N_DOCS="${N_DOCS:-100000}"
PHASES="${PHASES:-base}"
PROFILE="${PROFILE:-full}"

# Parse --profile=fast from CLI args
for arg in "$@"; do
  case "$arg" in
    --profile=*) PROFILE="${arg#--profile=}" ;;
    --profile) ;;
  esac
done

if [ "$PROFILE" = "fast" ]; then
  N_DOCS=10000
  LOG="$DIR/run_fast.log"
  ENGINE_TIMEOUT=120
else
  LOG="$DIR/run.log"
fi

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
  QDRANT__STORAGE__STORAGE_PATH="$QDRANT_DATA" QDRANT__SERVICE__HTTP_PORT=6333 "$QDRANT_BIN" > "$DIR/qdrant.log" 2>&1 &
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

# Allow skipping engines whose results already exist
SKIP_ENGINES="${SKIP_ENGINES:-}"
SUFFIX="full"
if [ "${DRY_RUN}" = "1" ]; then SUFFIX="dry"; fi
if [ "$PROFILE" = "fast" ]; then SUFFIX="fast"; fi
# Auto-skip engines with existing full results
AUTO_SKIP=""
IFS=',' read -ra _CHECK_LIST <<< "$ALL_ENGINES"
for _e in "${_CHECK_LIST[@]}"; do
  if [ -f "$DIR/results/${_e}_${SUFFIX}.jsonl" ]; then
    AUTO_SKIP="${AUTO_SKIP:+$AUTO_SKIP,}$_e"
    echo "[run] Skipping $_e (result exists: results/${_e}_${SUFFIX}.jsonl)"
  fi
done
SKIP_ENGINES="${SKIP_ENGINES:+$SKIP_ENGINES,}$AUTO_SKIP"

DRY_FLAG=""
if [ "$DRY_RUN" = "1" ]; then
  DRY_FLAG="--dry-run"
  echo "[run] DRY RUN MODE (1k docs)"
fi

PHASES_FLAG="--phases=$PHASES"
PROFILE_FLAG="--profile=$PROFILE"

echo "[run] Engines: $ALL_ENGINES"
echo "[run] Phases: $PHASES"
echo "[run] Profile: $PROFILE"
echo "[run] Starting benchmark at $(date)"

# Run each engine as separate process to avoid segfault from mixed .so loading
# No -e: a segfault in one engine should not stop the rest
IFS=',' read -ra ENGINE_LIST <<< "$ALL_ENGINES"
ENGINE_TIMEOUT="${ENGINE_TIMEOUT:-1200}"  # 20min per engine default (overridden by fast profile above)

for engine in "${ENGINE_LIST[@]}"; do
  # Skip if in SKIP_ENGINES
  if echo "$SKIP_ENGINES" | grep -qE "(^|,)${engine}(,|$)"; then
    echo "[run] --- $engine SKIPPED ---"
    continue
  fi
  echo "[run] --- $engine ---"
  set +e
  timeout "$ENGINE_TIMEOUT" python3 "$DIR/bench.py" \
    --engine "$engine" \
    --n-docs "$N_DOCS" \
    $DRY_FLAG \
    $PHASES_FLAG \
    $PROFILE_FLAG 2>&1 | tee -a "$LOG"
  EXIT_CODE=${PIPESTATUS[0]}
  set -e
  if [ $EXIT_CODE -eq 124 ]; then
    echo "[warn] engine=$engine TIMEOUT after ${ENGINE_TIMEOUT}s, continuing..."
  elif [ $EXIT_CODE -ne 0 ]; then
    echo "[warn] engine=$engine exited with code=$EXIT_CODE (segfault=139), continuing..."
  fi
done

echo "[run] Benchmark complete at $(date)"

# Aggregate and generate report
if [ "$PROFILE" = "fast" ]; then
  python3 "$DIR/report.py" --profile=fast
  echo "[run] Report written to $DIR/RESULTS_FAST.md"
else
  python3 "$DIR/report.py" --partial
  echo "[run] Report written to $DIR/RESULTS.md"
fi
