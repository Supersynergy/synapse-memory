#!/usr/bin/env bash
# Synapse Telepathy — SessionStart + UserPromptSubmit hook.
# Injects top-N recent activity from OTHER live Claude Code sessions.
set -euo pipefail

INPUT=$(cat)
MY_SID=$(printf '%s' "$INPUT" | python3 -c "
import json,sys
try:
    d=json.load(sys.stdin); print((d.get('session_id') or '')[:8])
except Exception:
    print('')
" 2>/dev/null || true)

HITS=$(${SYN_BIN:-syn} search "telepathy" 2>/dev/null | head -40)
[ -z "${HITS:-}" ] && exit 0

FILTERED=$(printf '%s\n' "$HITS" \
    | grep '\[telepathy\]' \
    | grep -v "\[${MY_SID}\]" \
    | awk '!seen[$0]++' \
    | head -6)
[ -z "${FILTERED:-}" ] && exit 0

echo "## 📡 Telepathy — recent activity from other sessions"
printf '%s\n' "$FILTERED" | sed 's/^/- /'
