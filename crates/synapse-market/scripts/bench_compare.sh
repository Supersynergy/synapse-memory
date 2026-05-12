#!/usr/bin/env bash
set -euo pipefail

# bench_compare.sh — run benches, parse output, compare to bench-baselines.toml
# Exits 1 if any baseline regressed >10%.
# Usage: ./scripts/bench_compare.sh [--no-run] [bench_output_dir]
#   --no-run : skip cargo bench, just parse existing *.txt in bench_output_dir
#   bench_output_dir : directory with bench-*.txt files (default: .)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BASELINES="$CRATE_DIR/bench-baselines.toml"

NO_RUN=0
OUT_DIR="."
for arg in "$@"; do
  case "$arg" in
    --no-run) NO_RUN=1 ;;
    *) OUT_DIR="$arg" ;;
  esac
done

PASS=0
FAIL=0

log_pass() { echo "  PASS  $1"; ((PASS+=1)); }
log_fail() { echo "  FAIL  $1"; ((FAIL+=1)); }
log_skip() { echo "  SKIP  $1 (no output)"; }

# ── helpers ──────────────────────────────────────────────────────────────────

criterion_median_us() {
  # Parse criterion "time:   [low µs  MEDIAN µs  high µs]" from file $1
  # Prints first median found, or empty string
  local f="$1"
  python3 - "$f" << 'EOF'
import re, sys
txt = open(sys.argv[1]).read()
m = re.search(r'time:\s+\[\S+\s+(?:µs|ns|ms)\s+([\d.]+)\s+(µs|ns|ms)', txt)
if not m:
    # alternate: [low  MEDIAN  high] all same unit on one line
    m = re.search(r'time:\s+\[[\d.]+ (µs|ns|ms)\s+[\d.]+ (µs|ns|ms)\s+([\d.]+) (µs|ns|ms)', txt)
    if not m:
        sys.exit(0)
    val, unit = float(m.group(3)), m.group(4)
else:
    val, unit = float(m.group(1)), m.group(2)
if unit == 'ns': val /= 1000
elif unit == 'ms': val *= 1000
print(f"{val:.3f}")
EOF
}

read_baseline() {
  # read_baseline <section> <key>  — prints value or empty
  python3 - "$BASELINES" "$1" "$2" << 'EOF'
import sys
try:
    import tomllib
except ImportError:
    import tomli as tomllib  # fallback
path, section, key = sys.argv[1], sys.argv[2], sys.argv[3]
with open(path, "rb") as f:
    data = tomllib.load(f)
val = data.get(section, {}).get(key)
if val is not None:
    print(val)
EOF
}

check_ceiling() {
  # check_ceiling <label> <measured_us> <baseline_us_max>
  local label="$1" measured="$2" ceiling="$3"
  if python3 -c "import sys; sys.exit(0 if float('$measured') <= float('$ceiling') * 1.10 else 1)"; then
    log_pass "$label: ${measured}µs ≤ ${ceiling}µs ceiling"
  else
    log_fail "$label: ${measured}µs > ${ceiling}µs ceiling (REGRESSION)"
  fi
}

check_floor() {
  # check_floor <label> <measured> <baseline_min>
  local label="$1" measured="$2" floor="$3"
  if python3 -c "import sys; sys.exit(0 if float('$measured') >= float('$floor') * 0.90 else 1)"; then
    log_pass "$label: ${measured} ≥ ${floor} floor"
  else
    log_fail "$label: ${measured} < ${floor} floor (REGRESSION)"
  fi
}

# ── run benches ───────────────────────────────────────────────────────────────

run_bench() {
  local name="$1"; shift
  local out="$OUT_DIR/bench-${name}.txt"
  if [[ $NO_RUN -eq 0 ]]; then
    echo "Running bench: $name ..."
    cargo bench --bench "$name" -p synapse-market 2>&1 | tee "$out"
  fi
  echo "$out"
}

# ── w1_candle_range ──────────────────────────────────────────────────────────
echo "=== w1_candle_range"
W1_OUT=$(run_bench w1_candle_range)
if [[ -f "$W1_OUT" ]]; then
  median=$(criterion_median_us "$W1_OUT")
  if [[ -n "$median" ]]; then
    ceiling=$(read_baseline w1_candle_range held_handle_p50_us)
    [[ -n "$ceiling" ]] && check_ceiling "w1_candle_range held_handle_p50_us" "$median" "$ceiling"
  else
    log_skip "w1_candle_range (no criterion output)"
  fi
fi

# ── w3_simd_agg ──────────────────────────────────────────────────────────────
echo "=== w3_simd_agg"
W3_OUT=$(run_bench w3_simd_agg)
if [[ -f "$W3_OUT" ]]; then
  speedup=$(python3 - "$W3_OUT" << 'EOF'
import re, sys
txt = open(sys.argv[1]).read()
m = re.search(r'A1.*SIMD speedup vs naive:\s+([\d.]+)\s*[×x]', txt)
if m: print(m.group(1))
EOF
)
  if [[ -n "$speedup" ]]; then
    floor=$(read_baseline w3_a1_mean_simd speedup_min)
    [[ -n "$floor" ]] && check_floor "w3_a1_mean_simd speedup" "$speedup" "$floor"
  else
    log_skip "w3_simd_agg A1 speedup (no matching line)"
  fi
fi

# ── amx_minimal ──────────────────────────────────────────────────────────────
echo "=== amx_minimal"
AMX_OUT=$(run_bench amx_minimal)
if [[ -f "$AMX_OUT" ]]; then
  amx_us=$(python3 - "$AMX_OUT" << 'EOF'
import re, sys
txt = open(sys.argv[1]).read()
m = re.search(r'cblas.*?([\d.]+)\s+ms', txt, re.IGNORECASE)
if m:
    print(float(m.group(1)) * 1000)
    sys.exit(0)
matches = re.findall(r'time:\s+\[\S+\s+(?:µs|ns|ms)\s+([\d.]+)\s+(µs|ns|ms)', txt)
if matches:
    val, unit = float(matches[0][0]), matches[0][1]
    if unit == 'ns': val /= 1000
    elif unit == 'ms': val *= 1000
    print(val)
EOF
)
  if [[ -n "$amx_us" ]]; then
    ceiling=$(read_baseline amx_corr_220 p50_us_max)
    [[ -n "$ceiling" ]] && check_ceiling "amx_corr_220 p50" "$amx_us" "$ceiling"
  else
    log_skip "amx_minimal (no timing found)"
  fi
fi

# ── hotset_point ─────────────────────────────────────────────────────────────
echo "=== hotset_point"
HS_OUT=$(run_bench hotset_point)
if [[ -f "$HS_OUT" ]]; then
  warm_p50=$(python3 - "$HS_OUT" << 'EOF'
import re, sys
txt = open(sys.argv[1]).read()
# "warm  p50=   0µs  p95=   1µs  [HotSet]"
m = re.search(r'\[HotSet\].*?p50=\s*([\d]+)µs|p50=\s*([\d]+)µs.*?\[HotSet\]', txt)
if not m:
    m = re.search(r'warm.*?p50=\s*([\d]+)', txt, re.IGNORECASE)
if m:
    print(m.group(1) or m.group(2))
EOF
)
  if [[ -n "$warm_p50" ]]; then
    ceiling=$(read_baseline hotset_warm p50_us_max)
    [[ -n "$ceiling" ]] && check_ceiling "hotset_warm p50" "$warm_p50" "$ceiling"
  else
    log_skip "hotset_point warm p50 (no matching line)"
  fi
fi

# ── jit_filter ───────────────────────────────────────────────────────────────
echo "=== jit_filter"
JIT_OUT=$(run_bench jit_filter)
if [[ -f "$JIT_OUT" ]]; then
  naive_us=$(python3 - "$JIT_OUT" "naive_rust" << 'EOF'
import re, sys
txt = open(sys.argv[1]).read()
# find the bench section for naive_rust
pat = re.compile(r'naive_rust.*?time:\s+\[[\d.]+\s+(?:µs|ns|ms)\s+([\d.]+)\s+(µs|ns|ms)', re.DOTALL)
m = pat.search(txt)
if m:
    val, unit = float(m.group(1)), m.group(2)
    if unit == 'ns': val /= 1000
    elif unit == 'ms': val *= 1000
    print(val)
EOF
)
  if [[ -n "$naive_us" ]]; then
    ceiling=$(read_baseline jit_filter naive_us_max)
    [[ -n "$ceiling" ]] && check_ceiling "jit_filter naive_us" "$naive_us" "$ceiling"
  fi
fi

# ── pattern_throughput ───────────────────────────────────────────────────────
echo "=== pattern_throughput"
PAT_OUT=$(run_bench pattern_throughput)
if [[ -f "$PAT_OUT" ]]; then
  # Criterion reports throughput: "thrpt:  [19.123 Melem/s  20.456 Melem/s  21.789 Melem/s]"
  vs_out=$(python3 - "$PAT_OUT" << 'EOF'
import re, sys
txt = open(sys.argv[1]).read()
results = {}
for m in re.finditer(r'(volume_spike_1M|cluster_1M).*?thrpt:\s+\[[\d.]+\s+\S+\s+([\d.]+)\s+(Kelem/s|Melem/s|elem/s)', txt, re.DOTALL):
    name, val, unit = m.group(1), float(m.group(2)), m.group(3)
    if unit == 'Kelem/s': val *= 1_000
    elif unit == 'Melem/s': val *= 1_000_000
    results[name] = val
for k, v in results.items():
    print(f"{k}={v:.0f}")
EOF
)
  if [[ -n "$vs_out" ]]; then
    while IFS='=' read -r name val; do
      case "$name" in
        volume_spike_1M)
          floor=$(read_baseline pattern_fsm volumespike_events_per_sec_min)
          [[ -n "$floor" ]] && check_floor "pattern_fsm volume_spike" "$val" "$floor"
          ;;
        cluster_1M)
          floor=$(read_baseline pattern_fsm insidercluster_events_per_sec_min)
          [[ -n "$floor" ]] && check_floor "pattern_fsm insider_cluster" "$val" "$floor"
          ;;
      esac
    done <<< "$vs_out"
  else
    log_skip "pattern_throughput (no thrpt lines)"
  fi
fi

# ── summary ───────────────────────────────────────────────────────────────────
echo ""
echo "=== bench_compare summary: PASS=$PASS  FAIL=$FAIL"
if [[ $FAIL -gt 0 ]]; then
  echo "REGRESSION DETECTED — $FAIL gate(s) failed"
  exit 1
fi
echo "All gates passed."
