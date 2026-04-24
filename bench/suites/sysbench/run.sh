#!/usr/bin/env bash
# sysbench OLTP against Synapse-MySQL (:3309) and vanilla MySQL (:13308)
# Credentials: root/wpbench, db: wordpress_c
set -euo pipefail

HOST=127.0.0.1
PORT_SYNAPSE=3309
PORT_MYSQL=13308
USER=root
PASS=wpbench
DB=wordpress_c
TABLES=4
TABLE_SIZE=10000
THREADS_LIST=(1 8)
WORKLOADS=(oltp_read_only oltp_read_write oltp_point_select)

run_bench() {
  local label=$1 port=$2
  echo "=== $label (port $port) ==="
  for wl in "${WORKLOADS[@]}"; do
    echo "--- prepare $wl ---"
    sysbench "$wl" \
      --mysql-host="$HOST" --mysql-port="$port" \
      --mysql-user="$USER" --mysql-password="$PASS" \
      --mysql-db="$DB" \
      --tables="$TABLES" --table-size="$TABLE_SIZE" \
      prepare 2>&1
    for t in "${THREADS_LIST[@]}"; do
      echo "--- run $wl threads=$t ---"
      sysbench "$wl" \
        --mysql-host="$HOST" --mysql-port="$port" \
        --mysql-user="$USER" --mysql-password="$PASS" \
        --mysql-db="$DB" \
        --tables="$TABLES" --table-size="$TABLE_SIZE" \
        --threads="$t" --time=30 --report-interval=10 \
        run 2>&1
    done
    echo "--- cleanup $wl ---"
    sysbench "$wl" \
      --mysql-host="$HOST" --mysql-port="$port" \
      --mysql-user="$USER" --mysql-password="$PASS" \
      --mysql-db="$DB" \
      --tables="$TABLES" --table-size="$TABLE_SIZE" \
      cleanup 2>&1
  done
}

run_bench "MySQL-8.0-vanilla" "$PORT_MYSQL"
run_bench "Synapse-MySQL-proxy" "$PORT_SYNAPSE"
