#!/usr/bin/env bash
# UserPromptSubmit — native `synx context-hook` first: zero-token trigger gate,
# cited context pack, and lockfile freshness in one Rust call. The Python
# pipeline remains the fallback when no synx binary is installed (or an old one
# that predates `context-hook`).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PY="${PYTHON:-python3}"

if command -v synx >/dev/null 2>&1 && synx context-hook </dev/null >/dev/null 2>&1; then
  synx context-hook 2>/dev/null || true
else
  # Freshness/version guard works without the daemon; recall itself fails open.
  "$PY" "$SCRIPT_DIR/synapse_context.py" --mode prompt 2>/dev/null || true
fi
