#!/usr/bin/env bash
# oltp_fair.sh — Fair sysbench OLTP comparison: Synapse async vs MariaDB/MySQL.
#
# Workload: oltp_point_select, 10k rows in sbtest1, threads={1,4,8,16}, 10s each.
# - sysbench's libmariadb in 1.0.20 hard-requires a TLS-capable server, so we
#   ship a throwaway self-signed cert to Synapse and let the client pick
#   plaintext via --mysql-ssl=off (the daemon then uses run_on plain path).
# - Reference server: whatever responds on 3306 (MariaDB if installed, else
#   MySQL); skipped if unreachable.
#
# Output:
#   bench/results/2026-04-25/oltp-fair.json
#   bench/results/2026-04-25/oltp-fair.md
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/synapse-mysql-async"
OUTDIR="$ROOT/bench/results/2026-04-25"
mkdir -p "$OUTDIR"

SYN_PORT=13312
SYN_DB=/tmp/synapse-oltp-fair.db
TLS_DIR=/tmp/synapse-bench-tls
TLS_CERT="$TLS_DIR/cert.pem"
TLS_KEY="$TLS_DIR/key.pem"

mkdir -p "$TLS_DIR"
if [ ! -s "$TLS_CERT" ] || [ ! -s "$TLS_KEY" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 30 \
    -subj "/CN=synapse-bench" \
    -keyout "$TLS_KEY" -out "$TLS_CERT" >/dev/null 2>&1
fi

# ── MariaDB/MySQL detection (5s timeout) ─────────────────────────────────────
REF_HOST=127.0.0.1
REF_PORT=3306
REF_USER=root
REF_PASS=""
REF_FLAVOR=""
REF_VER=$(timeout 5 mysql --protocol=TCP -h "$REF_HOST" -P "$REF_PORT" -u "$REF_USER" \
  --connect-timeout=3 -BNe 'SELECT VERSION()' 2>/dev/null || true)
if echo "$REF_VER" | grep -qi mariadb; then REF_FLAVOR=mariadb
elif echo "$REF_VER" | grep -qE '^[0-9]'; then REF_FLAVOR=mysql
fi
echo "ref flavor: ${REF_FLAVOR:-none} (version=${REF_VER:-?})"

# ── Synapse daemon (re)start ─────────────────────────────────────────────────
pkill -f "synapse-mysql-async.*:${SYN_PORT}" 2>/dev/null || true
sleep 1
rm -f "$SYN_DB"
nohup "$BIN" -f "$SYN_DB" -b "127.0.0.1:${SYN_PORT}" --mode strict --pool-size 32 \
  --tls-cert "$TLS_CERT" --tls-key "$TLS_KEY" \
  > /tmp/synx-async-fair.log 2>&1 &
disown
sleep 2
if ! lsof -iTCP:${SYN_PORT} -sTCP:LISTEN >/dev/null 2>&1; then
  echo "ERROR: Synapse daemon failed to bind ${SYN_PORT}"; tail /tmp/synx-async-fair.log; exit 1
fi
echo "synapse daemon up on ${SYN_PORT} (pid=$(lsof -tiTCP:${SYN_PORT} -sTCP:LISTEN | head -1))"

# ── Common sysbench args ─────────────────────────────────────────────────────
SB_COMMON=(--db-driver=mysql --tables=1 --table-size=10000 --mysql-ssl=off)

prepare_target () {
  local host="$1" port="$2" user="$3" pass="$4" db="$5"
  timeout 60 sysbench oltp_point_select "${SB_COMMON[@]}" \
    --mysql-host="$host" --mysql-port="$port" \
    --mysql-user="$user" --mysql-password="$pass" --mysql-db="$db" \
    cleanup >/dev/null 2>&1 || true
  timeout 120 sysbench oltp_point_select "${SB_COMMON[@]}" \
    --mysql-host="$host" --mysql-port="$port" \
    --mysql-user="$user" --mysql-password="$pass" --mysql-db="$db" \
    prepare >/dev/null 2>&1
}

run_one () {
  local label="$1" host="$2" port="$3" user="$4" pass="$5" db="$6" thr="$7"
  local raw
  raw=$(timeout 30 sysbench oltp_point_select "${SB_COMMON[@]}" \
    --mysql-host="$host" --mysql-port="$port" \
    --mysql-user="$user" --mysql-password="$pass" --mysql-db="$db" \
    --threads="$thr" --time=10 --report-interval=0 run 2>&1) || true
  local tps qps p95
  tps=$(awk '/transactions:/{gsub(/[()]/,""); print $3; exit}' <<<"$raw")
  qps=$(awk '/queries:/{gsub(/[()]/,""); print $3; exit}' <<<"$raw")
  p95=$(awk '/95th percentile/{print $3; exit}' <<<"$raw")
  printf '{"target":"%s","threads":%d,"tps":%s,"qps":%s,"p95_ms":%s}' \
    "$label" "$thr" "${tps:-0}" "${qps:-0}" "${p95:-0}"
}

JSON="$OUTDIR/oltp-fair.json"
THREADS=(1 4 8 16)
LINES=()

# Synapse
echo "preparing synapse..."
prepare_target 127.0.0.1 "$SYN_PORT" root synapse synapse
for t in "${THREADS[@]}"; do
  echo -n "  synapse t=$t ... "
  line=$(run_one "synapse" 127.0.0.1 "$SYN_PORT" root synapse synapse "$t")
  echo "$line"
  LINES+=("$line")
done

# Reference
if [ -n "$REF_FLAVOR" ]; then
  timeout 5 mysql --protocol=TCP -h "$REF_HOST" -P "$REF_PORT" -u "$REF_USER" \
    -e 'CREATE DATABASE IF NOT EXISTS sbtest_fair' 2>/dev/null || true
  echo "preparing $REF_FLAVOR..."
  prepare_target "$REF_HOST" "$REF_PORT" "$REF_USER" "$REF_PASS" sbtest_fair || \
    { echo "  prepare FAILED — skipping ref runs"; REF_FLAVOR=""; }
  for t in "${THREADS[@]}"; do
    [ -z "$REF_FLAVOR" ] && break
    echo -n "  $REF_FLAVOR t=$t ... "
    line=$(run_one "$REF_FLAVOR" "$REF_HOST" "$REF_PORT" "$REF_USER" "$REF_PASS" sbtest_fair "$t")
    echo "$line"
    LINES+=("$line")
  done
fi

# Emit JSON
{
  echo "["
  for i in "${!LINES[@]}"; do
    printf "  %s" "${LINES[$i]}"
    if [ "$i" -lt "$((${#LINES[@]}-1))" ]; then echo ","; else echo ""; fi
  done
  echo "]"
} > "$JSON"

# Markdown
MD="$OUTDIR/oltp-fair.md"
{
  echo "# OLTP Fair Bench — Synapse vs ${REF_FLAVOR:-(no ref)}"
  echo ""
  echo "Date: 2026-04-25"
  echo "Host: $(uname -srm)"
  echo "Workload: \`sysbench oltp_point_select\` (10k rows, 10s, threads {1,4,8,16})"
  echo ""
  echo "## Caveats"
  echo "- Synapse advertises a self-signed TLS cert because sysbench's libmariadb"
  echo "  refuses to dial a non-TLS server even with \`--mysql-ssl=off\`. Client"
  echo "  selects plaintext via that flag → wire path identical to a real"
  echo "  TLS-disabled deployment."
  echo "- Synapse backend: SQLite WAL + 512 MB mmap + 64 MB page cache."
  echo "- Prepared-statement LRU bumped to 256 (rusqlite \`prepare_cached\`),"
  echo "  collapses repeat-parse cost on the hot \`SELECT c FROM sbtest1 WHERE id=?\`."
  echo "- Single host, loopback only, no network noise."
  echo "- Ref server: \`${REF_FLAVOR:-none}\` ($REF_VER) on ${REF_HOST}:${REF_PORT}."
  echo ""
  echo "## Results"
  echo ""
  echo "| target | threads | tps | qps | p95 ms |"
  echo "|---|---:|---:|---:|---:|"
  python3 -c "
import json
data = json.load(open('$JSON'))
for r in data:
    print(f\"| {r['target']} | {r['threads']} | {r['tps']} | {r['qps']} | {r['p95_ms']} |\")
"
  echo ""
  echo "## Artifacts"
  echo "- raw: \`$JSON\`"
  echo "- daemon log: \`/tmp/synx-async-fair.log\`"
} > "$MD"

echo ""
echo "wrote $JSON"
echo "wrote $MD"
