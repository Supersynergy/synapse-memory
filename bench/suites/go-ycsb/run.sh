#!/usr/bin/env bash
# go-ycsb workloads A/B/C/D/F against Synapse-MySQL (:3309) and MySQL (:13308)
set -euo pipefail

GOYCSB=$(go env GOPATH)/bin/go-ycsb
if [ ! -x "$GOYCSB" ]; then
  echo "Installing go-ycsb..."
  go install github.com/pingcap/go-ycsb/cmd/go-ycsb@latest
fi

HOST=127.0.0.1
PORT_SYNAPSE=3309
PORT_MYSQL=13308
USER=root
PASS=wpbench
DB=wordpress_c
RECORD_COUNT=10000
OP_COUNT=10000

WORKLOADS=(workloada workloadb workloadc workloadf)

run_ycsb() {
  local label=$1 port=$2
  echo "=== $label (port $port) ==="
  DSN="${USER}:${PASS}@tcp(${HOST}:${port})/${DB}"

  echo "--- load ---"
  "$GOYCSB" load mysql \
    -P "$GOYCSB_WORKLOAD_DIR/workloada" \
    -p "mysql.dsn=$DSN" \
    -p "recordcount=$RECORD_COUNT" \
    -p "operationcount=$OP_COUNT" \
    2>&1

  for wl in "${WORKLOADS[@]}"; do
    for t in 1 8; do
      echo "--- run $wl threads=$t ---"
      "$GOYCSB" run mysql \
        -P "$GOYCSB_WORKLOAD_DIR/$wl" \
        -p "mysql.dsn=$DSN" \
        -p "recordcount=$RECORD_COUNT" \
        -p "operationcount=$OP_COUNT" \
        -p "threadcount=$t" \
        2>&1
    done
  done
}

GOYCSB_WORKLOAD_DIR="$(go env GOPATH)/pkg/mod/github.com/pingcap/go-ycsb@*/workloads"
GOYCSB_WORKLOAD_DIR=$(ls -d $GOYCSB_WORKLOAD_DIR 2>/dev/null | head -1)
if [ -z "$GOYCSB_WORKLOAD_DIR" ]; then
  # fallback: find in GOPATH src
  GOYCSB_WORKLOAD_DIR="$(go env GOPATH)/pkg/mod/github.com/pingcap/go-ycsb*/workloads"
fi

run_ycsb "MySQL-8.0-vanilla" "$PORT_MYSQL"
run_ycsb "Synapse-MySQL-proxy" "$PORT_SYNAPSE"
