#!/usr/bin/env bash
set -euo pipefail

# test_all.sh — full end-to-end test + bench pipeline for synapse-market
# Usage: ./scripts/test_all.sh [--skip-wipe] [--skip-bench]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
START_TS=$(date +%s)

SKIP_WIPE=0
SKIP_BENCH=0
for arg in "$@"; do
  case "$arg" in
    --skip-wipe)  SKIP_WIPE=1 ;;
    --skip-bench) SKIP_BENCH=1 ;;
  esac
done

cd "$CRATE_DIR"

echo "======================================================================"
echo " synapse-market full test pipeline"
echo " crate: $CRATE_DIR"
echo " date : $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "======================================================================"

# ── 1. wipe target ────────────────────────────────────────────────────────────
if [[ $SKIP_WIPE -eq 0 ]]; then
  echo ""
  echo "── [1/5] wipe target/ ──────────────────────────────────────────────────"
  cargo clean -p synapse-market
  echo "Done."
fi

# ── 2. build ──────────────────────────────────────────────────────────────────
echo ""
echo "── [2/5] cargo build (release) ─────────────────────────────────────────"
RUSTC_WRAPPER="" cargo build -p synapse-market --release 2>&1
echo "Done."

# ── 3. tests ─────────────────────────────────────────────────────────────────
echo ""
echo "── [3/5] cargo test ────────────────────────────────────────────────────"
RUSTC_WRAPPER="" cargo test -p synapse-market --no-fail-fast 2>&1 | tee /tmp/smx-test-output.txt
TEST_PASS=$(grep -c "^test .* ok$" /tmp/smx-test-output.txt || true)
TEST_FAIL=$(grep -c "^test .* FAILED$" /tmp/smx-test-output.txt || true)
TEST_IGNORE=$(grep -c "^test .* ignored$" /tmp/smx-test-output.txt || true)
echo "Tests: $TEST_PASS pass, $TEST_FAIL fail, $TEST_IGNORE ignored"
if [[ $TEST_FAIL -gt 0 ]]; then
  echo "ERROR: $TEST_FAIL test(s) failed!"
  exit 1
fi

# ── 4. benches ───────────────────────────────────────────────────────────────
BENCH_REPORT_FILE="/tmp/smx-bench-report.md"
if [[ $SKIP_BENCH -eq 0 ]]; then
  echo ""
  echo "── [4/5] cargo bench ───────────────────────────────────────────────────"
  BENCH_OUTPUT_DIR="/tmp/smx-bench-outputs"
  mkdir -p "$BENCH_OUTPUT_DIR"

  BENCHES=(
    w1_candle_range
    w3_simd_agg
    amx_minimal
    pattern_throughput
    jit_filter
    hotset_point
  )

  for bench in "${BENCHES[@]}"; do
    echo "  Bench: $bench ..."
    RUSTC_WRAPPER="" cargo bench --bench "$bench" -p synapse-market 2>&1 \
      | tee "$BENCH_OUTPUT_DIR/bench-${bench}.txt" \
      || echo "  WARN: $bench bench failed (non-fatal)"
  done

  echo ""
  echo "── [4b] bench regression compare ──────────────────────────────────────"
  bash "$SCRIPT_DIR/bench_compare.sh" --no-run "$BENCH_OUTPUT_DIR" \
    | tee /tmp/smx-bench-compare.txt || {
      echo "ERROR: Regression gate FAILED — see /tmp/smx-bench-compare.txt"
      exit 1
    }

  # Generate markdown report
  {
    echo "# Bench Report — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo ""
    echo "## Test Summary"
    echo ""
    echo "| metric | value |"
    echo "|--------|-------|"
    echo "| tests pass | $TEST_PASS |"
    echo "| tests fail | $TEST_FAIL |"
    echo "| tests ignored | $TEST_IGNORE |"
    echo ""
    echo "## Bench Regression Gate"
    echo '```'
    cat /tmp/smx-bench-compare.txt
    echo '```'
    echo ""
    echo "## Raw Bench Output"
    for bench in "${BENCHES[@]}"; do
      f="$BENCH_OUTPUT_DIR/bench-${bench}.txt"
      if [[ -f "$f" ]]; then
        echo ""
        echo "### $bench"
        echo '```'
        grep -E "time:|thrpt:|PASS|FAIL|speedup|p50" "$f" || true
        echo '```'
      fi
    done
  } > "$BENCH_REPORT_FILE"

  echo ""
  echo "Bench report: $BENCH_REPORT_FILE"
fi

# ── 5. summary ────────────────────────────────────────────────────────────────
END_TS=$(date +%s)
DURATION=$((END_TS - START_TS))

echo ""
echo "======================================================================"
echo " SUMMARY"
echo "======================================================================"
echo "  Build   : OK"
echo "  Tests   : $TEST_PASS pass  $TEST_FAIL fail  $TEST_IGNORE ignored"
if [[ $SKIP_BENCH -eq 0 ]]; then
  GATE_RESULT=$(grep "All gates passed\|REGRESSION" /tmp/smx-bench-compare.txt 2>/dev/null || echo "unknown")
  echo "  Benches : $GATE_RESULT"
  echo "  Report  : $BENCH_REPORT_FILE"
fi
echo "  Duration: ${DURATION}s"
echo "======================================================================"
