#!/usr/bin/env bash
# synapse-doctor — health check for Synapse installation
# Usage: bash docs/getting-started/doctor.sh
set -uo pipefail

PASS=0; FAIL=0; WARN=0

ok()   { echo "  [PASS] $*"; PASS=$((PASS+1)); }
fail() { echo "  [FAIL] $*"; FAIL=$((FAIL+1)); }
warn() { echo "  [WARN] $*"; WARN=$((WARN+1)); }

echo "=== Synapse Doctor ==="
echo ""

# 1. synapse CLI
echo "--- CLI ---"
if command -v synapse &>/dev/null; then
  VER=$(synapse --version 2>/dev/null || echo "unknown")
  ok "synapse found: $VER"
else
  fail "synapse not in PATH — run: cargo install synapse-cli"
fi

# 2. synapsql
echo "--- synapsql ---"
if command -v synapsql &>/dev/null; then
  ok "synapsql found"
else
  warn "synapsql not in PATH (optional, needed for MySQL mode)"
fi

# 3. synapse-mcp
echo "--- synapse-mcp ---"
if command -v synapse-mcp &>/dev/null; then
  ok "synapse-mcp found"
else
  warn "synapse-mcp not in PATH (optional, needed for Claude/Cursor MCP)"
fi

# 4. Rust toolchain
echo "--- Rust ---"
if command -v cargo &>/dev/null; then
  RUST_VER=$(rustc --version 2>/dev/null || echo "unknown")
  ok "cargo found: $RUST_VER"
else
  warn "cargo not found (needed to build from source)"
fi

# 5. Python + maturin
echo "--- Python ---"
if command -v python3 &>/dev/null; then
  PY_VER=$(python3 --version 2>/dev/null)
  ok "python3 found: $PY_VER"
  if python3 -c "import maturin" 2>/dev/null; then
    ok "maturin importable"
  else
    warn "maturin not installed (pip install maturin — needed for Python bindings)"
  fi
  if python3 -c "import synapse_rs" 2>/dev/null; then
    ok "synapse_rs Python module importable"
  else
    warn "synapse_rs not built — run: maturin develop -p synapse-py --release"
  fi
else
  warn "python3 not found (optional)"
fi

# 6. Smoke test: init + put + search
echo "--- Smoke test ---"
TMPDB=$(mktemp /tmp/synapse-doctor-XXXXXX.db)
trap "rm -f '$TMPDB'" EXIT

if synapse init -f "$TMPDB" &>/dev/null && \
   synapse put  -f "$TMPDB" --text "smoke test" &>/dev/null && \
   synapse find -f "$TMPDB" "smoke" 2>/dev/null | grep -q "smoke"; then
  ok "init + put + find smoke test"
else
  fail "smoke test failed — synapse may be broken"
fi

if synapse hybrid -f "$TMPDB" "smoke test" 2>/dev/null | grep -q "smoke"; then
  ok "hybrid search smoke test"
else
  warn "hybrid search returned no results (embeddings may be disabled)"
fi

# 7. MCP socket
echo "--- MCP socket ---"
SOCK="${SYNAPSE_SOCK:-/tmp/synapse.sock}"
if [[ -S "$SOCK" ]]; then
  ok "MCP socket live: $SOCK"
else
  warn "MCP socket not found at $SOCK (start: synapse-mcp --sock $SOCK --db ./brain.db)"
fi

echo ""
echo "=== Result: $PASS passed, $WARN warnings, $FAIL failed ==="
[[ $FAIL -eq 0 ]] && exit 0 || exit 1
