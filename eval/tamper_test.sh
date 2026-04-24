#!/usr/bin/env bash
# Persona #9 — Ed25519 tamper-flip test
# 5 variants: header, payload-byte, signature-byte, length-field, version
# Each: sign a brainpack, flip 1 byte, verify MUST fail via restore
# Honest result: restore only checks magic bytes today; sig check not implemented in restore path
set -euo pipefail

SYNAPSE=/Users/master/projects/synapse/target/release/synapse
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

DB="$TMPDIR/test.db"
DB2="$TMPDIR/test2.db"
BP="$TMPDIR/test.brainpack"
TAMPED="$TMPDIR/tampered.brainpack"

# --- setup: create, sign a brainpack ---
$SYNAPSE init -f "$DB"
$SYNAPSE -f "$DB" put --text "sensitive compliance document about financial regulations"

cd "$TMPDIR"
$SYNAPSE keygen 2>/dev/null

if [ -f "$TMPDIR/synapse.sk" ]; then
  $SYNAPSE -f "$DB" snap-signed --sk "$TMPDIR/synapse.sk" "$BP"
  SIGNED=true
else
  $SYNAPSE -f "$DB" snap "$BP"
  SIGNED=false
fi

SZ=$(wc -c < "$BP")
echo "Brainpack size: $SZ bytes  signed=$SIGNED"
echo ""

pass=0
fail_count=0

run_variant() {
  local name="$1"
  local offset="$2"
  local fresh_db="$TMPDIR/fresh_${name//[^a-z]/_}.db"

  cp "$BP" "$TAMPED"
  python3 -c "
data = bytearray(open('$TAMPED','rb').read())
idx = $offset % len(data)
orig = data[idx]
data[idx] ^= 0xFF
open('$TAMPED','wb').write(bytes(data))
print(f'  flip offset={idx:#06x} 0x{orig:02x}→0x{data[idx]:02x}', flush=True)
"

  RESTORE_OUT=$($SYNAPSE -f "$fresh_db" restore "$TAMPED" 2>&1 || true)
  RESTORE_EXIT=$?

  if echo "$RESTORE_OUT" | grep -qi "invalid\|corrupt\|tamper\|mismatch\|bad magic\|checksum\|fail\|error" \
      || [ $RESTORE_EXIT -ne 0 ]; then
    echo "PASS  $name — tamper detected ✓"
    ((pass++)) || true
  else
    echo "FAIL  $name — tamper NOT detected ✗  (restore succeeded silently; sig-check gap)"
    ((fail_count++)) || true
  fi
}

run_variant "header-byte"        4
run_variant "payload-byte"       $((SZ / 4))
run_variant "signature-byte"     $((SZ - 32))
run_variant "length-field"       8
run_variant "version-magic-byte" 0

echo ""
echo "Results: $pass/5 PASS  |  $fail_count/5 FAIL"
echo ""
echo "NOTE: Only magic-byte flip (variant 5) is caught by restore today."
echo "      Variants 1-4 expose a real gap: restore does not verify Ed25519"
echo "      over the brainpack payload. Fix = PR-F1 style: add VK param to restore,"
echo "      verify sig before deserialize. This test will become 5/5 PASS after that PR."

if [ $pass -eq 5 ]; then
  echo "STATUS: PASS — all 5 tamper variants detected"
  exit 0
else
  echo "STATUS: PARTIAL — $pass/5 detected (honest gap documented)"
  exit 1
fi
