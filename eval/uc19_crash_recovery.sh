#!/usr/bin/env bash
# UC19 — daemon crash recovery smoke
# 1. Ingest 200 docs into a temp db (lib/file mode, fast)
# 2. Start synapsed pointing at same db
# 3. kill -9 daemon mid-idle
# 4. Restart daemon, verify doc-count + search preserved (WAL durability)
# Exit 0 = PASS, 1 = FAIL
set -euo pipefail

SYNAPSE=/Users/master/projects/synapse/target/release/synapse
SYNAPSED=/Users/master/projects/synapse/target/release/synapsed
TMPDIR=$(mktemp -d)
SOCK="$TMPDIR/synapse.sock"
DB="$TMPDIR/uc19.db"

trap "kill \${DAEMON_PID:-0} 2>/dev/null || true; rm -rf $TMPDIR" EXIT

echo "[UC19] initializing db..."
$SYNAPSE init -f "$DB"

echo "[UC19] ingesting 200 docs..."
for i in $(seq 1 200); do
  $SYNAPSE put -f "$DB" --no-embed --text "crash-test doc $i topic $((i % 20))" >/dev/null
done

BEFORE=$($SYNAPSE stats -f "$DB" 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('docs',0))" 2>/dev/null || echo "?")
echo "[UC19] docs before kill: $BEFORE"

# Start daemon
$SYNAPSED -f "$DB" -s "$SOCK" --lazy-embed &
DAEMON_PID=$!
sleep 0.4

# kill -9 while idle
kill -9 $DAEMON_PID
sleep 0.2

# Restart daemon
$SYNAPSED -f "$DB" -s "$SOCK" --lazy-embed &
DAEMON_PID=$!
sleep 0.4

AFTER=$($SYNAPSE stats -f "$DB" 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('docs',0))" 2>/dev/null || echo "?")
echo "[UC19] docs after restart: $AFTER"

SEARCH_OUT=$($SYNAPSE find -f "$DB" "crash" 2>/dev/null | wc -l || echo "0")
SEARCH_OK=0
[ "${SEARCH_OUT// /}" -gt 0 ] && SEARCH_OK=1

if [ "$AFTER" = "$BEFORE" ] && [ "$AFTER" != "?" ] && [ "$SEARCH_OK" = "1" ]; then
  echo "[UC19] PASS — docs preserved ($AFTER/$BEFORE), search ok"
  exit 0
else
  echo "[UC19] FAIL — before=$BEFORE after=$AFTER search_ok=$SEARCH_OK"
  exit 1
fi
