#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

python3 -m py_compile products/freshness-router/files/hooks/synapse_context.py
python3 -m py_compile \
  products/freshness-router/scripts/register-primary-sources.py \
  products/freshness-router/scripts/render-obsidian-status.py
test -s products/freshness-router/files/QUICKSTART.md
python3 products/freshness-router/scripts/register-primary-sources.py >/dev/null
cargo test -p synapse-core fresh::tests
cargo check -p synapse-cli
SYNX_BIN="${SYNX_BIN:-$ROOT/target/release/synx}"
test -x "$SYNX_BIN"
"$SYNX_BIN" fresh-evidence \
  products/freshness-router/files/evidence-example.json \
  --now 1785756600 --require-current >/dev/null

echo "freshness-router verify PASS"
