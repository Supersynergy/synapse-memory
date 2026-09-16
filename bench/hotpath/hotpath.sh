#!/usr/bin/env bash
# hotpath.sh — reproducible synx hot-path benchmark.
#
# Measures on a throwaway brain (default 5k docs, --no-embed):
#   should-context predicate, context/prime cold (no daemon) vs warm
#   (synapsed socket), maintain telemetry, and the WAL-bloat regression:
#   cold open with a fat un-checkpointed WAL vs after `maintain` truncates it.
#
# Usage:
#   bench/hotpath/hotpath.sh [--docs N] [--wal-mb M] [--reps R] [--out FILE]
# Env: SYNX / SYNAPSED override binary lookup. Missing synapsed → warm section
# is skipped (results marked null, not failed).
set -euo pipefail

DOCS=5000
WAL_MB=64
REPS=3
OUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --docs) DOCS="$2"; shift 2;;
    --wal-mb) WAL_MB="$2"; shift 2;;
    --reps) REPS="$2"; shift 2;;
    --out) OUT="$2"; shift 2;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done

SYNX="${SYNX:-$(command -v synx || echo "$HOME/.local/bin/synx")}"
SYNAPSED="${SYNAPSED:-$(command -v synapsed || echo "$HOME/.local/bin/synapsed")}"
[ -x "$SYNX" ] || { echo "synx not found: $SYNX" >&2; exit 1; }

WORK="$(mktemp -d /tmp/synapse-hotpath.XXXXXX)"
BRAIN="$WORK/brain.db"
SOCK="$WORK/synapse.sock"
DAEMON_PID=""
PAD_PID=""

cleanup() {
  [ -n "$DAEMON_PID" ] && kill "$DAEMON_PID" 2>/dev/null || true
  [ -n "$PAD_PID" ] && kill "$PAD_PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# Median of N runs in milliseconds (python3: portable sub-second timing).
t_ms() {
  python3 - "$REPS" "$@" <<'PY'
import subprocess, sys, time, statistics
reps, cmd = int(sys.argv[1]), sys.argv[2:]
ts = []
for _ in range(reps):
    t0 = time.monotonic()
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    ts.append((time.monotonic() - t0) * 1000)
print(int(statistics.median(ts)))
PY
}

echo "== brain: $DOCS docs --no-embed =="
python3 - "$SYNX" "$BRAIN" "$DOCS" <<'PY'
import subprocess, sys
synx, brain, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
words = ("retrieval context memory bandit wal sqlite daemon latency "
         "embedding index shard tenant fusion decay fresh").split()
for i in range(n):
    txt = f"doc {i} " + " ".join(words[(i + j) % len(words)] for j in range(24))
    subprocess.run([synx, "-f", brain, "put", "--text", txt, "--no-embed"],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
PY

Q="retrieval memory index"
export SYNAPSE_DEBUG=1

echo "== cold (no daemon) =="
COLD_CTX="$(t_ms env SYNAPSE_NO_DAEMON=1 "$SYNX" -f "$BRAIN" context "$Q" --limit 5)"
COLD_PRIME="$(t_ms env SYNAPSE_NO_DAEMON=1 "$SYNX" -f "$BRAIN" prime)"
SHOULD="$(t_ms env SYNAPSE_NO_DAEMON=1 "$SYNX" -f "$BRAIN" should-context "$Q" || true)"

WARM_CTX=null; WARM_PRIME=null
if [ -x "$SYNAPSED" ]; then
  echo "== warm (synapsed) =="
  "$SYNAPSED" -f "$BRAIN" -s "$SOCK" --wal-checkpoint-secs 0 \
      >"$WORK/daemon.log" 2>&1 &
  DAEMON_PID=$!
  for _ in $(seq 1 60); do [ -S "$SOCK" ] && break; sleep 0.5; done
  [ -S "$SOCK" ] || { echo "daemon socket never appeared" >&2; exit 1; }
  # warm-up call (loads index/embedder) — not timed
  env SYNAPSE_SOCK="$SOCK" "$SYNX" -f "$BRAIN" context "$Q" --limit 5 >/dev/null 2>&1 || true
  WARM_CTX="$(t_ms env SYNAPSE_SOCK="$SOCK" "$SYNX" -f "$BRAIN" context "$Q" --limit 5)"
  WARM_PRIME="$(t_ms env SYNAPSE_SOCK="$SOCK" "$SYNX" -f "$BRAIN" prime)"
else
  echo "== warm: skipped (no synapsed binary) =="
fi

echo "== WAL regression ($WAL_MB MiB pad, un-checkpointed) =="
python3 - "$BRAIN" "$WAL_MB" "$WORK/pad.ready" <<'PY' &
import sqlite3, sys, time
brain, mb, ready = sys.argv[1], int(sys.argv[2]), sys.argv[3]
c = sqlite3.connect(brain)
c.execute("PRAGMA wal_autocheckpoint=0")
c.execute("CREATE TABLE IF NOT EXISTS bench_pad(t TEXT)")
blob = "x" * 65536
rows = max(1, mb * 16)          # 16 rows x 64KiB ≈ 1 MiB WAL per unit
c.executemany("INSERT INTO bench_pad VALUES (?)", [(blob,)] * rows)
c.commit()
open(ready, "w").write("ok")
time.sleep(600)                 # hold conn open → WAL cannot collapse
PY
PAD_PID=$!
for _ in $(seq 1 120); do [ -f "$WORK/pad.ready" ] && break; sleep 0.25; done
WAL_BEFORE=$(stat -f%z "$BRAIN-wal" 2>/dev/null || echo 0)
# The real cliff is crash recovery, not routine opens: kill -9 the holder and
# drop the shared wal-index so the next open rebuilds it over the full WAL.
kill -9 "$PAD_PID" 2>/dev/null || true
PAD_PID=""
rm -f "$BRAIN-shm"
OPEN_FAT="$(t_ms env SYNAPSE_NO_DAEMON=1 "$SYNX" -f "$BRAIN" context "$Q" --limit 5)"

MAINT_JSON="$(env SYNAPSE_NO_DAEMON=1 "$SYNX" -f "$BRAIN" maintain --json 2>/dev/null || echo '{}')"
WAL_AFTER=$(stat -f%z "$BRAIN-wal" 2>/dev/null || echo 0)
OPEN_LEAN="$(t_ms env SYNAPSE_NO_DAEMON=1 "$SYNX" -f "$BRAIN" context "$Q" --limit 5)"

RESULT="$(python3 - <<PY
import json
print(json.dumps({
  "docs": $DOCS,
  "reps": $REPS,
  "ms": {
    "should_context": $SHOULD,
    "context_cold_no_daemon": $COLD_CTX,
    "prime_cold_no_daemon": $COLD_PRIME,
    "context_warm_daemon": $WARM_CTX,
    "prime_warm_daemon": $WARM_PRIME,
    "context_cold_fat_wal": $OPEN_FAT,
    "context_cold_after_maintain": $OPEN_LEAN
  },
  "wal_bytes": {"before_maintain": $WAL_BEFORE, "after_maintain": $WAL_AFTER},
  "maintain": json.loads('''$MAINT_JSON''')
}, indent=2))
PY
)"
echo "$RESULT"
if [ -n "$OUT" ]; then
  printf '%s\n' "$RESULT" > "$OUT"
  echo "wrote $OUT" >&2
fi
