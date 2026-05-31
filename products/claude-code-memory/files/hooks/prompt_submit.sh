#!/usr/bin/env bash
# UserPromptSubmit — budgeted Synapse recall with adaptive context packing.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PY="${PYTHON:-python3}"

# Freshness/version guard works without the daemon; recall itself fails open.
"$PY" "$SCRIPT_DIR/synapse_context.py" --mode prompt 2>/dev/null || true
