#!/usr/bin/env bash
# fair_durability_linux.sh — io_uring durability bench (bare-metal Linux only)
# Requires: Ubuntu 24.04+, kernel >=6.1, python3 + sqlite3 module
# Usage: bash scripts/fair_durability_linux.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BENCH="$SCRIPT_DIR/bench-dashboard/fair_durability_bench.py"
OUT="$SCRIPT_DIR/bench-dashboard/FAIR_DURABILITY_BENCH_LINUX_$(date +%Y-%m-%d).md"

if [[ "$(uname)" != "Linux" ]]; then
    echo "ERROR: This script must run on bare-metal Linux (not macOS, not colima)."
    echo "colima overlay-fs adds 2-5× latency — results would be unfair."
    exit 1
fi

# Check for io_uring support
if ! python3 -c "import liburing" 2>/dev/null; then
    echo "NOTE: liburing Python bindings not found. Running SQLite + in-mem only."
    echo "      For io_uring benches: pip install liburing (or use Rust storage bench)."
fi

echo "=== FAIR DURABILITY BENCH (Linux bare-metal) ==="
echo "Kernel: $(uname -r)"
echo "CPU:    $(grep 'model name' /proc/cpuinfo | head -1 | cut -d: -f2 | xargs)"
echo ""

# Run base Python bench (SQLite + in-mem)
python3 "$BENCH"

echo ""
echo "io_uring Rust bench:"
echo "  cd $SCRIPT_DIR && cargo bench --bench fair_durability_iouring 2>/dev/null || echo 'Rust bench not yet wired (see bench/suites/)'"
echo ""
echo "Output: $OUT"
