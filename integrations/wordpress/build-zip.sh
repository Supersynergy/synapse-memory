#!/usr/bin/env bash
# build-zip.sh — package synapsql plugin for wordpress.org submission
set -euo pipefail

cd "$(dirname "$0")"
VERSION=$(grep '^Stable tag:' synapsql/readme.txt | awk '{print $3}')
OUT="../../target/synapsql-${VERSION}.zip"
mkdir -p ../../target

rm -f "$OUT"
cd synapsql
zip -r "$OUT" . \
  -x ".*" "*.DS_Store" "node_modules/*" "*.log"
cd ..

echo "✓ packaged: $OUT"
unzip -l "$OUT" | tail -5
echo
echo "Submit to wordpress.org via SVN:"
echo "  svn checkout https://plugins.svn.wordpress.org/synapsql synapsql-svn"
echo "  cp -r synapsql/* synapsql-svn/trunk/"
echo "  svn add synapsql-svn/trunk/* --force"
echo "  svn commit -m 'release ${VERSION}'"
