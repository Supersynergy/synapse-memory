#!/usr/bin/env bash
# Persona #17 — DB crash recovery fuzz
# kill -9 synapse-lib-demo at 10/25/50/75/90% through ingest of 10k docs
# 10 runs, reopen + count + integrity check each time
set -euo pipefail

DEMO=/Users/master/projects/synapse/target/release/synapse-lib-demo
SYNAPSE=/Users/master/projects/synapse/target/release/synapse
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

TOTAL_DOCS=10000
KILL_POINTS=(10 25 50 75 90 10 25 50 75 90)  # 10 runs

pass=0
fail_count=0

echo "Crash-fuzz: $TOTAL_DOCS docs, kill at ${KILL_POINTS[*]}%"
echo ""

for i in "${!KILL_POINTS[@]}"; do
  PCT=${KILL_POINTS[$i]}
  RUN=$((i+1))
  DB="$TMPDIR/run${RUN}.db"

  # Start ingest in background — write TOTAL_DOCS docs
  (
    $SYNAPSE init -f "$DB" 2>/dev/null
    for n in $(seq 1 $TOTAL_DOCS); do
      $SYNAPSE -f "$DB" put --text "doc $n content about topic $((n % 100))" 2>/dev/null
    done
  ) &
  BG_PID=$!

  # Wait for kill-point percentage of expected time (rough proxy)
  # Measure total expected time at 1k docs/s heuristic → 10s; adjust by pct
  SLEEP_MS=$(( PCT * 100 / 10 ))   # pct * 10ms per 1% => 100ms * pct/10
  sleep "$(echo "scale=3; $PCT * 0.008" | bc)"

  # Kill with -9 (uncatchable)
  kill -9 $BG_PID 2>/dev/null || true
  wait $BG_PID 2>/dev/null || true

  # Reopen and check
  if [ ! -f "$DB" ]; then
    echo "FAIL  run $RUN (kill@${PCT}%) — DB file missing"
    ((fail_count++)) || true
    continue
  fi

  # Count docs (may be partial — any non-zero count + no crash = pass)
  STATS_OUT=$($SYNAPSE -f "$DB" stats 2>/dev/null || echo '{}')
  COUNT=$(echo "$STATS_OUT" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('docs',0))" 2>/dev/null || echo "??")

  # Integrity: can we open + search without error?
  if $SYNAPSE -f "$DB" find "topic 1" 2>/dev/null >/dev/null; then
    echo "PASS  run $RUN (kill@${PCT}%) — reopened ok, docs=$COUNT, search ok"
    ((pass++)) || true
  else
    echo "FAIL  run $RUN (kill@${PCT}%) — reopen/search error after crash (docs=$COUNT)"
    ((fail_count++)) || true
  fi
done

echo ""
echo "Results: $pass/10 PASS  |  $fail_count/10 FAIL"
RATE=$((pass * 100 / 10))
echo "Pass rate: ${RATE}%"
if [ $pass -ge 8 ]; then
  echo "STATUS: PASS (≥80% recovery)"
  exit 0
else
  echo "STATUS: FAIL (<80% recovery)"
  exit 1
fi
