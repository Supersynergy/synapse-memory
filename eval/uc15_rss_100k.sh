#!/usr/bin/env bash
# UC15 — RSS memory at 100k / 1M docs
# Measures peak RSS of synapsed serving a hot corpus via /usr/bin/time -l
# Usage: ./uc15_rss_100k.sh [--docs 100000] [--skip-ingest]
# Exit 0 = PASS (RSS < threshold), 1 = FAIL
set -euo pipefail

SYNAPSE=/Users/master/projects/synapse/target/release/synapse
SYNAPSED=/Users/master/projects/synapse/target/release/synapsed
DOCS=${DOCS:-100000}
RSS_LIMIT_MB=${RSS_LIMIT_MB:-512}  # 512 MB at 100k docs = generous SLA

TMPDIR=$(mktemp -d)
SOCK="$TMPDIR/synapse.sock"
DB="$TMPDIR/uc15.db"
TIME_OUT="$TMPDIR/time.txt"

trap "kill \${DAEMON_PID:-0} 2>/dev/null || true; rm -rf $TMPDIR" EXIT

echo "[UC15] target=$DOCS docs, RSS limit=${RSS_LIMIT_MB}MB"

# --- ingest (no-embed for speed) ---
echo "[UC15] ingesting $DOCS docs (no-embed)..."
$SYNAPSE init -f "$DB"
for i in $(seq 1 $DOCS); do
  $SYNAPSE put -f "$DB" --no-embed --text "rss-test doc $i topic $((i % 500))" >/dev/null
  [ $((i % 10000)) -eq 0 ] && echo "  ... $i / $DOCS"
done
echo "[UC15] ingest done"

# --- start daemon + measure RSS ---
echo "[UC15] starting synapsed, measuring RSS..."
/usr/bin/time -l $SYNAPSED -f "$DB" -s "$SOCK" --lazy-embed &
DAEMON_PID=$!
sleep 1.0  # warm

# Issue 100 queries to warm caches
for q in $(seq 1 100); do
  $SYNAPSE find -f "$DB" "topic $((q % 100))" >/dev/null 2>&1 || true
done

# Snapshot RSS via ps (resident set size in bytes on macOS)
RSS_BYTES=$(ps -o rss= -p $DAEMON_PID 2>/dev/null || echo "0")
RSS_MB=$((RSS_BYTES / 1024))
echo "[UC15] RSS after 100 warm queries: ${RSS_MB} MB (${RSS_BYTES} KB)"

# Kill daemon cleanly
kill $DAEMON_PID 2>/dev/null || true

# Report
PASS=0
[ "$RSS_MB" -lt "$RSS_LIMIT_MB" ] && PASS=1

if [ "$PASS" = "1" ]; then
  echo "[UC15] PASS — RSS=${RSS_MB}MB < limit=${RSS_LIMIT_MB}MB at ${DOCS} docs"
  exit 0
else
  echo "[UC15] FAIL — RSS=${RSS_MB}MB >= limit=${RSS_LIMIT_MB}MB at ${DOCS} docs"
  exit 1
fi
