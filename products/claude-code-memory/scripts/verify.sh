#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

python3 -m py_compile \
  products/claude-code-memory/files/hooks/synapse_context.py \
  products/claude-code-memory/files/telepathy/daemon.py

PYTHONPATH=sdk/python python3 -m pytest -q integrations/claude-code/hooks/test_synapse_context.py

echo "claude-code-memory verify PASS"
