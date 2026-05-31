#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d -t synapse-three-session.XXXXXX)"
SOCK="$WORK/synapse.sock"
DB="$WORK/brain.db"
LOG="$WORK/synapsed.log"

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]]; then
    kill "$DAEMON_PID" >/dev/null 2>&1 || true
    wait "$DAEMON_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

cd "$ROOT"
cargo build --quiet -p synapsed --bin synapsed --bin synx-fast
target/debug/synapsed --file "$DB" --sock "$SOCK" --lazy-embed >"$LOG" 2>&1 &
DAEMON_PID=$!

for _ in {1..100}; do
  [[ -S "$SOCK" ]] && break
  sleep 0.05
done
[[ -S "$SOCK" ]] || { cat "$LOG" >&2; exit 1; }

export SYNAPSE_SOCK="$SOCK"

cat >"$WORK/app.py" <<'PY'
def normalize_user_id(value):
    return value
PY

target/debug/synx-fast put \
  --title "demo/session-1/decision" \
  --meta '{"scope":"demo/three-session-bugfix","kind":"decision"}' \
  --no-embed \
  "Session 1 decision: normalize_user_id must strip whitespace and lowercase IDs before database lookup."

target/debug/synx-fast put \
  --title "demo/session-2/failing-test" \
  --meta '{"scope":"demo/three-session-bugfix","kind":"test"}' \
  --no-embed \
  "Session 2 failure: User ID ' Alice ' misses cache because normalize_user_id returns raw input."

echo "Session 3 recall:"
RECALL_OUT="$(printf '%s\n' "normalize_user_id cache miss whitespace lowercase" \
  | target/debug/synx-fast batch --scope "demo/three-session-bugfix" find --limit 3)"
echo "$RECALL_OUT"
if ! grep -q "Session 1 decision" <<<"$RECALL_OUT" || ! grep -q "Session 2 failure" <<<"$RECALL_OUT"; then
  echo "FAIL: scoped recall did not return the prior decision and failing-test observation" >&2
  exit 1
fi

python3 - <<'PY' "$WORK/app.py"
from pathlib import Path
path = Path(__import__("sys").argv[1])
path.write_text("def normalize_user_id(value):\n    return value.strip().lower()\n")
PY

python3 - <<'PY' "$WORK/app.py"
import importlib.util, sys
spec = importlib.util.spec_from_file_location("app", sys.argv[1])
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
assert mod.normalize_user_id(" Alice ") == "alice"
print("PASS: session-3 fix used recalled decision and failing test")
PY

echo "Demo workspace: $WORK"
