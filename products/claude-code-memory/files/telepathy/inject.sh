#!/usr/bin/env bash
# Synapse Telepathy — SessionStart + UserPromptSubmit hook.
# Injects top-N recent activity from OTHER live Claude Code sessions.
set -euo pipefail

INPUT=$(cat)
MY_SID=$(printf '%s' "$INPUT" \
  | sed -n 's/.*"session_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  | cut -c1-8)

DB="${SYNAPSE_DB:-$HOME/.synapse/brain.db}"
MAX_LINES="${TELEPATHY_CONTEXT_LINES:-4}"
MAX_CHARS="${TELEPATHY_CONTEXT_CHARS:-220}"
MAX_TOKENS="${TELEPATHY_CONTEXT_TOKENS:-260}"
HITS=""
if command -v sqlite3 >/dev/null 2>&1 && [ -r "$DB" ]; then
  HITS=$(sqlite3 -readonly -noheader "$DB" \
    "SELECT replace(substr(d.text,1,700), char(10), ' ')
     FROM docs_fts
     JOIN docs d ON d.id = docs_fts.rowid
     WHERE docs_fts MATCH 'telepathy'
     ORDER BY docs_fts.rowid DESC
     LIMIT 20;" 2>/dev/null || true)
fi
if [ -z "${HITS:-}" ]; then
  HITS=$(${SYNX_BIN:-synx} find "telepathy" 80 2>/dev/null | head -80 || true)
fi
[ -z "${HITS:-}" ] && exit 0

FILTERED=$(printf '%s\n' "$HITS" \
    | grep '\[telepathy\]' \
    | grep -v "\[${MY_SID}\]" \
    | awk '!seen[$0]++')
[ -z "${FILTERED:-}" ] && exit 0

PACKED=$(printf '%s\n' "$FILTERED" \
  | awk -v max_lines="$MAX_LINES" -v max_chars="$MAX_CHARS" -v max_tokens="$MAX_TOKENS" '
      BEGIN { n=0; used=0 }
      {
        gsub(/[[:space:]]+/, " ");
        if (length($0) > max_chars) {
          $0 = substr($0, 1, max_chars - 3) "...";
        }
        est = int(length($0) / 4) + 8;
        if (used + est > max_tokens) next;
        print "- " $0;
        used += est;
        n += 1;
        if (n >= max_lines) exit;
      }')
[ -z "${PACKED:-}" ] && exit 0

echo "## Telepathy — recent activity from other sessions"
printf '%s\n' "$PACKED"
