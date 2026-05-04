#!/usr/bin/env bash
# 4-Stack Vector Bench orchestrator
# Usage: bash bench/4stack/run.sh [--out-dir DIR]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TS=$(date +%Y-%m-%dT%H-%M-%S)
DATE=$(date +%Y-%m-%d)
OUT_DIR="${1:-$SCRIPT_DIR/../results/4stack-$DATE}"
mkdir -p "$OUT_DIR"

QUERIES="$SCRIPT_DIR/queries.txt"
PYTHON="${PYTHON:-python3}"

echo "=== 4-Stack Vector Bench $TS ==="
echo "Output: $OUT_DIR"
echo ""

run_stack() {
    local stack=$1
    local script=$2
    local out="$OUT_DIR/stack_${stack}.json"
    echo -n "  Stack $stack ... "
    if $PYTHON "$script" > "$out" 2>/tmp/bench_stack_${stack}_err.txt; then
        available=$(python3 -c "import json,sys; d=json.load(open('$out')); print('YES' if d.get('available') else 'SKIP')" 2>/dev/null || echo "?")
        echo "$available  [$out]"
    else
        echo "ERROR (see /tmp/bench_stack_${stack}_err.txt)"
        echo '{"stack":"'"$stack"'","available":false,"reason":"measure script failed"}' > "$out"
    fi
}

echo "[1/4] Stack A — sqlite-vec via synapsed"
run_stack A "$SCRIPT_DIR/measure_a.py"

echo "[2/4] Stack B — Python turbo :9477"
run_stack B "$SCRIPT_DIR/measure_b.py"

echo "[3/4] Stack C — Core+ndarray Step-5 (skip if absent)"
run_stack C "$SCRIPT_DIR/measure_c.py"

echo "[4/4] Stack D — synapse-ultra (skip if absent)"
run_stack D "$SCRIPT_DIR/measure_d.py"

echo ""
echo "[5/5] Aggregating results..."
REPORT="$OUT_DIR/run-${TS}.md"
$PYTHON "$SCRIPT_DIR/aggregate.py" "$OUT_DIR" --ts "$TS" > "$REPORT" 2>/dev/null || \
    $PYTHON "$SCRIPT_DIR/aggregate.py" "$OUT_DIR" --ts "$TS"

echo ""
echo "=== Done ==="
echo "Report: $REPORT"
echo ""
cat "$REPORT"
