#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

python3 -m py_compile products/freshness-router/files/hooks/synapse_context.py
test -s products/freshness-router/files/QUICKSTART.md

echo "freshness-router verify PASS"
