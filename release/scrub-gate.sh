#!/usr/bin/env bash
# scrub-gate — refuses to let private data or protected IP into a public bundle.
# Scans a directory (a freshly-emitted OSS extract). Exit 0 = safe. Non-zero = BLOCK.
# Usage: release/scrub-gate.sh <dir>
set -uo pipefail
DIR="${1:?usage: scrub-gate.sh <dir>}"
fail=0
note() { printf '  %s %s\n' "$1" "$2"; }
echo "== scrub-gate :: $DIR =="

# 1. Private-data files that must never ship.
DATA=$(find "$DIR" -type f \( -iname '*.db' -o -iname '*.sqlite*' -o -iname '*.env' \
        -o -iname '*.key' -o -iname '*.pem' -o -iname 'brain.*' -o -iname '*.brainpack' \
        -o -iname '*.npy' \) 2>/dev/null)
[ -n "$DATA" ] && { note "❌ DATA" "private-data files:"; echo "$DATA" | sed 's/^/      /'; fail=1; } \
                || note "✅ DATA" "no private-data files"

# 2. Protected IP source must not be in the bundle.
IP=$(find "$DIR" -type d \( -name 'synapse-core' -o -name 'synapse-engine' -o -name 'synapse-license' \
        -o -name 'synapse-kernel' -o -name 'synapse-market' \) 2>/dev/null
     find "$DIR" -type f \( -iname 'LICENSE-ENGINE*' -o -iname 'LICENSE-CORE*' -o -iname '*watermark*' \) 2>/dev/null)
[ -n "$IP" ] && { note "❌ IP" "proprietary/FSL source present:"; echo "$IP" | sed 's/^/      /'; fail=1; } \
              || note "✅ IP" "no proprietary/FSL source"

# 3. Personal identifiers / private paths in tracked content.
ID=$(grep -rIl -E 'true@supersynergy\.de|/Users/master|/Users/[a-z]+/\.synapse' "$DIR" 2>/dev/null)
[ -n "$ID" ] && { note "❌ ID" "personal identifiers / private paths:"; echo "$ID" | head -10 | sed 's/^/      /'; fail=1; } \
              || note "✅ ID" "no personal identifiers"

# 4. Secret-shaped strings.
SEC=$(grep -rIl -E '(sk-[A-Za-z0-9]{20,})|(ghp_[A-Za-z0-9]{20,})|(AKIA[0-9A-Z]{16})|BEGIN (RSA|OPENSSH|EC) PRIVATE KEY' "$DIR" 2>/dev/null)
[ -n "$SEC" ] && { note "❌ SECRET" "secret-shaped strings:"; echo "$SEC" | head | sed 's/^/      /'; fail=1; } \
              || note "✅ SECRET" "no secret-shaped strings"

echo "== $( [ $fail -eq 0 ] && echo 'PASS — bundle safe to publish' || echo 'BLOCK — fix above' ) =="
exit $fail
