#!/bin/bash
# UserPromptSubmit — inject semantic memory if query matches known scope/project
[ ! -S /tmp/synapse.sock ] && exit 0
PROMPT=$(cat)
# Only inject for prompts >20 chars (skip acks)
[ ${#PROMPT} -lt 20 ] && exit 0
# Use first 200 chars of prompt as query
Q=$(echo "$PROMPT" | head -c 200)
OUT=$(syn hybrid "$Q" 3 2>/dev/null | awk -F'\t' '$1+0 > 0.05 {printf "- %s: %s\n", $2, substr($3,1,100)}')
[ -z "$OUT" ] && exit 0
echo "<synapse_context>"
echo "Top-3 relevant memories:"
echo "$OUT"
echo "</synapse_context>"
