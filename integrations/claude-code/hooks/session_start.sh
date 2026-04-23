#!/bin/bash
[ ! -S /tmp/synapse.sock ] && exit 0
PROJ=$(basename "${PWD:-$(pwd)}")
OUT=$(syn hybrid "$PROJ decisions stack preferences" 8 2>/dev/null)
[ -z "$OUT" ] && exit 0
echo "## Synapse memory (top-8 for: $PROJ)"
echo "$OUT" | awk -F'\t' '{printf "- %s (%.2f): %s\n", $2, $1, substr($3,1,120)}' | head -16
