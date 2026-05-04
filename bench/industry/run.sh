#!/usr/bin/env bash
# Industry ANN Benchmark — ann-benchmarks methodology
# Compares: synapse-ultra, usearch, qdrant, lance, sqlite-vec
# Usage: bash bench/industry/run.sh [--out-dir DIR] [--skip-gt]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TS=$(date +%Y-%m-%dT%H-%M-%S)
DATE=$(date +%Y-%m-%d)
OUT_DIR="${OUT_DIR:-$SCRIPT_DIR/../results/industry-${DATE}}"
SKIP_GT=0

for arg in "$@"; do
    case $arg in
        --out-dir=*) OUT_DIR="${arg#*=}" ;;
        --out-dir) shift; OUT_DIR="$1" ;;
        --skip-gt) SKIP_GT=1 ;;
    esac
done

mkdir -p "$OUT_DIR"
PYTHON="${PYTHON:-/opt/homebrew/bin/python3.12}"

echo "=== Industry ANN Benchmark $TS ==="
echo "Output: $OUT_DIR"
echo ""

# ── Step 0: Check dependencies ───────────────────────────────────────────────
echo "[0/6] Checking Python deps..."
$PYTHON -c "import sqlite_vec" 2>/dev/null || { echo "  MISSING: sqlite_vec"; exit 1; }
$PYTHON -c "import msgpack" 2>/dev/null || { echo "  MISSING: msgpack (pip install msgpack)"; exit 1; }
$PYTHON -c "import numpy" 2>/dev/null || { echo "  MISSING: numpy"; exit 1; }
echo "  Core deps OK"

# ── Step 1: Ground truth ──────────────────────────────────────────────────────
GT_BIN="$SCRIPT_DIR/ground_truth.bin"
if [ "$SKIP_GT" = "1" ] && [ -f "$GT_BIN" ]; then
    echo "[1/6] Ground truth exists — skipping (use without --skip-gt to regenerate)"
elif [ -f "$GT_BIN" ]; then
    echo "[1/6] Ground truth exists — skipping (delete ground_truth.bin to regenerate)"
else
    echo "[1/6] Generating ground truth (brute-force top-100 for 1000 queries)..."
    $PYTHON "$SCRIPT_DIR/01_ground_truth.py"
fi
echo ""

# ── Step 2-6: Engine benchmarks ───────────────────────────────────────────────
run_engine() {
    local label=$1
    local script=$2
    local out="$OUT_DIR/engine_${label}.json"
    echo -n "  [$label] $script ... "
    if $PYTHON "$script" > "$out" 2>/tmp/industry_${label}_err.txt; then
        avail=$($PYTHON -c "
import json,sys
d=json.load(open('$out'))
if isinstance(d,list): print('YES' if any(x.get('available') for x in d) else 'SKIP')
else: print('YES' if d.get('available') else 'SKIP')
" 2>/dev/null || echo "?")
        echo "$avail  [$out]"
    else
        echo "ERROR"
        cat /tmp/industry_${label}_err.txt | tail -5
        echo '{"engine":"'"$label"'","available":false,"reason":"bench script failed"}' > "$out"
    fi
}

echo "[2/6] sqlite-vec (brute-force, exact baseline)..."
run_engine "sqlite_vec" "$SCRIPT_DIR/bench_sqlite_vec.py"
echo ""

echo "[3/6] usearch direct (HNSW pure-Python)..."
run_engine "usearch" "$SCRIPT_DIR/bench_usearch.py"
echo ""

echo "[4/6] qdrant in-process (HNSW)..."
run_engine "qdrant" "$SCRIPT_DIR/bench_qdrant.py"
echo ""

echo "[5/6] lance (IVF_PQ)..."
run_engine "lance" "$SCRIPT_DIR/bench_lance.py"
echo ""

echo "[6/6] synapse-ultra (HTTP, binary_first + strict)..."
run_engine "ultra" "$SCRIPT_DIR/bench_ultra.py"
echo ""

# ── Aggregate ────────────────────────────────────────────────────────────────
echo "Aggregating results..."
REPORT="$OUT_DIR/run-${TS}.md"
$PYTHON "$SCRIPT_DIR/aggregate.py" "$OUT_DIR" "$TS" > "$REPORT" 2>/tmp/industry_agg_err.txt || {
    echo "  [warn] aggregation error:"
    cat /tmp/industry_agg_err.txt | tail -10
}

echo ""
echo "=== Done ==="
echo "Report: $REPORT"
echo ""
cat "$REPORT"
