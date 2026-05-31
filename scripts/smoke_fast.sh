#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo build -q -p synapsed --bin synapsed --bin synx-fast

tmp="$(mktemp -d)"
pid=""
cleanup() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

sock="$tmp/synapse.sock"
db="$tmp/brain.db"
./target/debug/synapsed --file "$db" --sock "$sock" >"$tmp/daemon.log" 2>&1 &
pid="$!"
export SYNAPSE_SOCK="$sock"

ready=0
for _ in {1..50}; do
  if ./target/debug/synx-fast ping >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.1
done
if [[ "$ready" != "1" ]]; then
  echo "daemon did not become ready"
  cat "$tmp/daemon.log"
  exit 1
fi

./target/debug/synx-fast doctor
./target/debug/synx-fast put --no-embed --scope synapse-smoke --title "scoped decision" \
  "Session decision: scoped recall keeps agent context precise" >/dev/null

hits="$(./target/debug/synx-fast scoped --mode lex --scope synapse-smoke "scoped recall" --limit 3)"
if [[ "$hits" != *"scoped recall"* ]]; then
  echo "scoped search smoke failed"
  echo "$hits"
  cat "$tmp/daemon.log"
  exit 1
fi

context="$(./target/debug/synx-fast context --mode lex --scope synapse-smoke "scoped recall" --budget 240)"
if [[ "$context" != *"<synapse_context"* || "$context" != *"scoped recall"* ]]; then
  echo "context smoke failed"
  echo "$context"
  cat "$tmp/daemon.log"
  exit 1
fi

batch="$(
  printf '%s\n' '{"title":"batch memory","text":"Batch scoped memory lets hooks avoid repeated process startup","embed":false}' |
    ./target/debug/synx-fast put-batch --scope synapse-smoke >/dev/null
  printf '%s\n' "Batch scoped memory" "scoped recall" |
    ./target/debug/synx-fast batch find --scope synapse-smoke --limit 3
)"
if [[ "$batch" != *"Batch scoped memory"* ]]; then
  echo "batch scoped smoke failed"
  echo "$batch"
  cat "$tmp/daemon.log"
  exit 1
fi

echo "smoke-fast PASS: doctor, scoped search, compact context, scoped batch"
