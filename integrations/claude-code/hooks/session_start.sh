#!/usr/bin/env bash
# SessionStart — native `synx prime` first: repo state + relevant memory in one
# startup brief. Python session mode remains the fallback without synx.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PY="${PYTHON:-python3}"
PROJ=$(basename "${PWD:-$(pwd)}")

if command -v synx >/dev/null 2>&1 && synx prime --help >/dev/null 2>&1; then
  synx prime . --limit 8 2>/dev/null || true
else
  # Freshness/version guard works without the daemon; recall itself fails open.
  printf '{"prompt":"","cwd":"%s"}' "${PWD:-$(pwd)}" \
    | "$PY" "$SCRIPT_DIR/synapse_context.py" --mode session --project "$PROJ" 2>/dev/null || true
fi
