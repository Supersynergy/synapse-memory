#!/usr/bin/env bash
# PHASE-4 WordPress SQL bench: MariaDB vs Synapse-MySQL
set -euo pipefail

SYNAPSE_BIN="/Users/master/projects/synapse/target/release/synapse-mysql"
SYNAPSE_DB="/tmp/synapse-wp-phase4.db"
SYNAPSE_PORT=13306
MARIADB_CONTAINER="wordpress-test-mariadb-1"
MARIADB_CLIENT="/opt/homebrew/Cellar/mariadb/12.2.2/bin/mariadb"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

log() { echo "[$(date +%H:%M:%S)] $*"; }

# Run SQL via docker exec (MariaDB)
docker_sql() {
  local sql=$1
  docker exec -i "$MARIADB_CONTAINER" \
    mariadb -uroot -psynapse wordpress --silent --skip-column-names \
    -e "$sql" 2>/dev/null
}

# Run SQL via mariadb client direct (Synapse on :SYNAPSE_PORT)
synapse_sql() {
  local sql=$1
  "$MARIADB_CLIENT" -h127.0.0.1 -P"$SYNAPSE_PORT" -uroot -psynapse --skip-ssl wordpress \
    --silent --skip-column-names -e "$sql" 2>/dev/null
}

# ── Start synapse-mysql ───────────────────────────────────────────────────────
start_synapse() {
  if lsof -nP -iTCP:${SYNAPSE_PORT} -sTCP:LISTEN &>/dev/null; then
    log "synapse-mysql already on :${SYNAPSE_PORT}"
    return
  fi
  log "Starting synapse-mysql :${SYNAPSE_PORT} db=${SYNAPSE_DB}"
  "$SYNAPSE_BIN" \
    --file "$SYNAPSE_DB" \
    --bind "127.0.0.1:${SYNAPSE_PORT}" \
    --root-password synapse \
    >> /tmp/synapse-mysql-phase4.log 2>&1 &
  echo $! > /tmp/synapse-mysql-phase4.pid
  sleep 2
  log "synapse-mysql started"
}

# ── Ensure MariaDB container running ─────────────────────────────────────────
ensure_mariadb() {
  if docker inspect "$MARIADB_CONTAINER" &>/dev/null; then
    local state
    state=$(docker inspect "$MARIADB_CONTAINER" --format '{{.State.Running}}')
    if [[ "$state" == "true" ]]; then
      log "MariaDB container running"
      # wait for ready
      for i in $(seq 1 20); do
        if docker exec -i "$MARIADB_CONTAINER" mariadb-admin ping -h localhost -psynapse &>/dev/null 2>&1; then
          log "MariaDB ready"
          return
        fi
        sleep 2
      done
      log "ERROR: MariaDB not ready"
      exit 1
    fi
  fi
  cd "$SCRIPT_DIR"
  log "Starting MariaDB container..."
  docker compose -f docker-compose-phase4.yml up -d mariadb
  for i in $(seq 1 30); do
    if docker exec -i "$MARIADB_CONTAINER" mariadb-admin ping -h localhost -psynapse &>/dev/null 2>&1; then
      log "MariaDB ready (${i}x2s)"
      return
    fi
    sleep 2
  done
  log "ERROR: MariaDB not ready after 60s"
  exit 1
}

# ── Time a query 3 times, echo "t1 t2 t3" ────────────────────────────────────
time_docker_query() {
  local sql=$1
  local times=()
  for _ in 1 2 3; do
    local t0 t1
    t0=$(python3 -c "import time; print(int(time.time()*1000))")
    "$MARIADB_CLIENT" -h127.0.0.1 -P13307 -uroot -psynapse --skip-ssl wordpress \
      --silent --skip-column-names -e "$sql" &>/dev/null || true
    t1=$(python3 -c "import time; print(int(time.time()*1000))")
    times+=($((t1 - t0)))
  done
  echo "${times[@]}"
}

time_synapse_query() {
  local sql=$1
  local times=()
  for _ in 1 2 3; do
    local t0 t1
    t0=$(python3 -c "import time; print(int(time.time()*1000))")
    "$MARIADB_CLIENT" -h127.0.0.1 -P"$SYNAPSE_PORT" -uroot -psynapse --skip-ssl wordpress \
      --silent --skip-column-names -e "$sql" &>/dev/null || true
    t1=$(python3 -c "import time; print(int(time.time()*1000))")
    times+=($((t1 - t0)))
  done
  echo "${times[@]}"
}

median3() { echo "$@" | tr ' ' '\n' | sort -n | sed -n '2p'; }

# ── SQL scenarios ─────────────────────────────────────────────────────────────
declare -A SQL_SCENARIOS
SQL_SCENARIOS[S1_home]="SELECT p.ID,p.post_title,pm.meta_value FROM wp_posts p LEFT JOIN wp_postmeta pm ON p.ID=pm.post_id AND pm.meta_key='_thumbnail_id' WHERE p.post_status='publish' AND p.post_type='post' ORDER BY p.post_date DESC LIMIT 10"
SQL_SCENARIOS[S2_single]="SELECT p.ID,p.post_title,p.post_content,pm.meta_value FROM wp_posts p JOIN wp_postmeta pm ON p.ID=pm.post_id WHERE p.post_status='publish' AND p.post_type='post' AND p.ID=42 LIMIT 1"
SQL_SCENARIOS[S3_search]="SELECT ID,post_title FROM wp_posts WHERE post_status='publish' AND MATCH(post_title,post_content) AGAINST('rust web framework' IN BOOLEAN MODE) LIMIT 10"
SQL_SCENARIOS[S4_admin]="SELECT p.ID,p.post_title,p.post_status,p.post_date,u.user_login FROM wp_posts p JOIN wp_users u ON p.post_author=u.ID WHERE p.post_type='post' ORDER BY p.post_date DESC LIMIT 25"
SQL_SCENARIOS[S5_wc]="SELECT p.ID,pm1.meta_value as price,pm2.meta_value as stock FROM wp_posts p JOIN wp_postmeta pm1 ON p.ID=pm1.post_id AND pm1.meta_key='_price' JOIN wp_postmeta pm2 ON p.ID=pm2.post_id AND pm2.meta_key='_stock_status' WHERE p.post_type='post' LIMIT 20"

# ── Bench both backends ───────────────────────────────────────────────────────
run_bench() {
  echo ""
  echo "### MariaDB 11 (direct client :13307)"
  printf "  %-14s %7s %7s %7s %9s\n" "Scenario" "Run1" "Run2" "Run3" "Median"
  printf "  %-14s %7s %7s %7s %9s\n" "--------" "-----" "-----" "-----" "------"
  for key in $(echo "${!SQL_SCENARIOS[@]}" | tr ' ' '\n' | sort); do
    local times
    times=$(time_docker_query "${SQL_SCENARIOS[$key]}")
    read -r t1 t2 t3 <<< "$times"
    local med; med=$(median3 $t1 $t2 $t3)
    printf "  %-14s %6dms %6dms %6dms %8dms\n" "$key" "$t1" "$t2" "$t3" "$med"
    eval "MB_${key}=${med}"
  done

  echo ""
  echo "### Synapse-MySQL (127.0.0.1:${SYNAPSE_PORT})"
  printf "  %-14s %7s %7s %7s %9s\n" "Scenario" "Run1" "Run2" "Run3" "Median"
  printf "  %-14s %7s %7s %7s %9s\n" "--------" "-----" "-----" "-----" "------"
  for key in $(echo "${!SQL_SCENARIOS[@]}" | tr ' ' '\n' | sort); do
    local times
    times=$(time_synapse_query "${SQL_SCENARIOS[$key]}")
    read -r t1 t2 t3 <<< "$times"
    local med; med=$(median3 $t1 $t2 $t3)
    printf "  %-14s %6dms %6dms %6dms %8dms\n" "$key" "$t1" "$t2" "$t3" "$med"
    eval "SN_${key}=${med}"
  done
}

print_comparison() {
  echo ""
  echo "## Comparison Table (median ms)"
  printf "%-14s %10s %10s %10s %12s\n" "Scenario" "MariaDB" "Synapse" "Speedup" "Target-SN"
  printf "%-14s %10s %10s %10s %12s\n" "--------" "-------" "-------" "-------" "---------"
  declare -A TGT; TGT[S1_home]=50; TGT[S2_single]=80; TGT[S3_search]=8; TGT[S4_admin]="-"; TGT[S5_wc]="-"

  for key in S1_home S2_single S3_search S4_admin S5_wc; do
    local mb sn speedup verdict tgt
    mb=$(eval echo "\${MB_${key}:-N/A}")
    sn=$(eval echo "\${SN_${key}:-N/A}")
    tgt="${TGT[$key]}"
    if [[ "$mb" != "N/A" && "$sn" != "N/A" && "$sn" -gt 0 ]]; then
      speedup=$(python3 -c "print(f'{$mb/$sn:.1f}×')" 2>/dev/null || echo "N/A")
    else
      speedup="N/A"
    fi
    if [[ "$tgt" == "-" ]]; then
      verdict="measure"
    elif [[ "$sn" != "N/A" && "$sn" -le "$tgt" ]]; then
      verdict="✓ PASS"
    else
      verdict="✗ MISS (t=${tgt}ms)"
    fi
    printf "%-14s %9sms %9sms %10s %12s\n" "$key" "$mb" "$sn" "$speedup" "$verdict"
  done
}

# ── MAIN ──────────────────────────────────────────────────────────────────────
main() {
  log "=== PHASE-4 WP Bench ==="

  start_synapse
  ensure_mariadb

  log "Seeding MariaDB (direct:13307)..."
  python3 "$SCRIPT_DIR/seed_wp.py" "direct:13307"

  log "Seeding Synapse (direct:${SYNAPSE_PORT})..."
  python3 "$SCRIPT_DIR/seed_wp.py" "direct:${SYNAPSE_PORT}"

  run_bench
  print_comparison

  log "Done."
}

main "$@"
