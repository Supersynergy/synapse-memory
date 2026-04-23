#!/bin/bash
# Stop — extract decisions/facts from last session tail, persist to synapse
# Reads session transcript via stdin if available, else skips
[ ! -S /tmp/synapse.sock ] && exit 0
INPUT=$(cat 2>/dev/null)
[ -z "$INPUT" ] && exit 0

PROJ=$(basename "${PWD:-$(pwd)}")
# Heuristic: grab lines with decision markers
EXTRACT=$(echo "$INPUT" | grep -iE "(chose|decided|picked|using|switched to|migrating to|replace.*with|over)" | head -5)
[ -z "$EXTRACT" ] && exit 0

# Put each line as scoped decision
echo "$EXTRACT" | while IFS= read -r line; do
  [ -z "$line" ] && continue
  line_trim=$(echo "$line" | sed 's/^[[:space:]]*//' | head -c 300)
  [ ${#line_trim} -lt 15 ] && continue
  echo "$line_trim" | syn put --title "session-extract/$PROJ" >/dev/null 2>&1
done
