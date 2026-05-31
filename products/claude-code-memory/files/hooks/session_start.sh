#!/usr/bin/env bash
# SessionStart — project-scoped Synapse warm context with a hard token budget.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PY="${PYTHON:-python3}"
PROJ=$(basename "${PWD:-$(pwd)}")

# Freshness/version guard works without the daemon; recall itself fails open.
printf '{"prompt":"","cwd":"%s"}' "${PWD:-$(pwd)}" \
  | "$PY" "$SCRIPT_DIR/synapse_context.py" --mode session --project "$PROJ" 2>/dev/null || true
