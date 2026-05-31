#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

python3 -m py_compile \
  products/benchmark-kit/files/recall_bakeoff/run.py \
  products/benchmark-kit/files/agentdb_public/bench_agentdb_public.py

test -s products/benchmark-kit/files/recall_bakeoff/results/latest.md
test -s products/benchmark-kit/files/agentdb_public/results/latest.md

echo "benchmark-kit verify PASS"
