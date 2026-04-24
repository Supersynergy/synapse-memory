#!/usr/bin/env bash
# Run every real-world bench in sequence — pass paths per scenario via env.
#
#   OBSIDIAN_VAULT=~/Vaults/MyNotes \
#   GMAIL_MBOX=~/Downloads/All-Mail.mbox \
#   SLACK_EXPORT=~/Downloads/SlackExport \
#   NOTES_DB=/tmp/notes.db \
#   LOGSEQ=~/Documents/Logseq \
#   CHATGPT_JSON=~/Downloads/conversations.json \
#   ./bench_all.sh
#
# Any unset var skips that bench.

set -uo pipefail
cd "$(dirname "$0")"

declare -a RAN=()
run() {
    local label="$1"; shift
    echo; echo "──── $label ────"
    if python "$@"; then
        RAN+=("$label")
    else
        echo "  (failed, skipping)"
    fi
}

[[ -n "${OBSIDIAN_VAULT-}" ]] && run "a01 Obsidian"       01_obsidian.py      "$OBSIDIAN_VAULT"
[[ -n "${LOGSEQ-}" ]]         && run "a04 Logseq"         04_logseq.py        "$LOGSEQ"
[[ -n "${NOTES_DB-}" ]]       && run "a03 Apple Notes"    03_apple_notes.py   "$NOTES_DB"
[[ -n "${CHATGPT_JSON-}" ]]   && run "b11 ChatGPT export" 11_chatgpt_history.py "$CHATGPT_JSON"
[[ -n "${GMAIL_MBOX-}" ]]     && run "c21 Gmail mbox"     21_gmail_mbox.py    "$GMAIL_MBOX"
[[ -n "${SLACK_EXPORT-}" ]]   && run "c22 Slack export"   22_slack_export.py  "$SLACK_EXPORT"

echo
echo "══ Summary ══"
if ((${#RAN[@]})); then
    printf "  ✓ %s\n" "${RAN[@]}"
else
    echo "  No scenarios ran — set at least one of OBSIDIAN_VAULT / LOGSEQ / NOTES_DB / CHATGPT_JSON / GMAIL_MBOX / SLACK_EXPORT."
fi
