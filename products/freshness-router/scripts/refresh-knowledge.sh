#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DB="${SYNAPSE_BRAIN_DB:-/Users/master/.synapse/brain.db}"
NOTE="${KNOWLEDGE_FRESHNESS_NOTE:-/Users/master/Documents/Obsidian Vault/03_Resources/Knowledge Freshness Control Plane.md}"
PYTHON="/opt/homebrew/bin/python3"
SYNX="/Users/master/projects/synapse/target/release/synx"
FRESHDOCS="/Users/master/.local/bin/freshdocs"
GUARD="/Users/master/.local/bin/run-guard"
STATUS=0

run_step() {
  "$@" || STATUS=1
}

run_step "$PYTHON" "$ROOT/products/freshness-router/scripts/register-primary-sources.py" \
  --file "$DB" --synx "$SYNX" >/dev/null
run_step "$GUARD" --timeout 900 --max-rss-gb 4 --threads 4 \
  "$SYNX" corpus sync-due --file "$DB" --limit 20 --embed
if "$FRESHDOCS" status --json | "$PYTHON" -c \
  'import json,sys; rows=json.load(sys.stdin); sys.exit(0 if any(row.get("fetched") is None or (row.get("age") or 0) >= 3 for row in rows) else 1)'
then
  run_step "$GUARD" --timeout 900 --max-rss-gb 4 --threads 4 \
    "$FRESHDOCS" sync --all
fi
run_step "$PYTHON" "$ROOT/products/freshness-router/scripts/render-obsidian-status.py" \
  --file "$DB" --note "$NOTE"

exit "$STATUS"
