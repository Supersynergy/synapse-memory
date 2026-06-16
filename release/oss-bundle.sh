#!/usr/bin/env bash
# oss-bundle — emit a clean public MIT subset of Synapse (the grounddb release) from
# the single private workspace. Ships ONLY `public-mit` crates from oss-manifest.toml,
# then runs the scrub-gate. Fails closed: a dirty bundle is never produced.
#
# Usage: release/oss-bundle.sh            # → release/dist/grounddb-oss/
#        release/oss-bundle.sh --check    # build manifest + scrub only, no copy
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
MANIFEST="$HERE/oss-manifest.toml"
OUT="$HERE/dist/grounddb-oss"

# Read the public-mit crate list from the manifest (tomllib, fail-closed).
crates=$(python3 - "$MANIFEST" <<'PY'
import sys, tomllib
m = tomllib.load(open(sys.argv[1], "rb"))
print("\n".join(m.get("tiers", {}).get("public-mit", {}).get("crates", [])))
PY
)
[ -z "$crates" ] && { echo "no public-mit crates in manifest — abort"; exit 1; }

echo "== oss-bundle: emitting public-mit subset =="
echo "$crates" | sed 's/^/  + /'

if [ "${1:-}" = "--check" ]; then
    echo "(--check: manifest parsed OK, $(echo "$crates" | wc -l | tr -d ' ') crates; no copy)"
    exit 0
fi

rm -rf "$OUT"; mkdir -p "$OUT/crates"
while read -r c; do
    [ -z "$c" ] && continue
    src="$ROOT/crates/$c"
    [ -d "$src" ] || { echo "MISSING crate $c — abort"; exit 1; }
    # copy source only; never vendored target/, caches, or data
    rsync -a --exclude target --exclude '*.db' --exclude '*.sqlite*' \
          --exclude data --exclude corpus --exclude '.env*' "$src/" "$OUT/crates/$c/"
done <<< "$crates"

# Sanitize the EMITTED copy (never the source): strip private home paths + owner email
# from text files so example configs/docs ship generic.
find "$OUT" -type f \( -name '*.md' -o -name '*.toml' -o -name '*.rs' -o -name '*.json' -o -name '*.txt' \) -print0 \
  | while IFS= read -r -d '' f; do
    sed -i '' \
      -e 's#/Users/[^/]*/BASE/projects/synapse#/path/to/grounddb#g' \
      -e 's#/Users/[^/]*/projects/synapse#/path/to/grounddb#g' \
      -e 's#/Users/[^/]*/\.synapse#~/.grounddb#g' \
      -e 's#true@supersynergy\.de#hello@grounddb.dev#g' \
      "$f" 2>/dev/null || true
  done

cp "$ROOT/LICENSE" "$OUT/LICENSE" 2>/dev/null || true
cat > "$OUT/NOTICE" <<'EOF'
grounddb — the open-source (MIT) subset of Synapse.
The proprietary SIMD engine, the FSL core, and private products are NOT included.
Emitted by release/oss-bundle.sh; verified by release/scrub-gate.sh.
EOF

echo "== running scrub-gate on the emitted bundle =="
bash "$HERE/scrub-gate.sh" "$OUT"
echo "== bundle ready: $OUT =="
