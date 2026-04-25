#!/usr/bin/env bash
# seed-license.sh — insert a test license row into the running license-server DB.
# Usage: seed-license.sh <license_key> <customer_id> [days_valid]
set -Eeuo pipefail
KEY="${1:?license_key required}"
CID="${2:?customer_id required}"
DAYS="${3:-30}"
DB="${SYN_LIC_DB:-/tmp/lic.db}"
EXP=$(date -u -v+"${DAYS}"d +%s 2>/dev/null || date -u -d "+${DAYS} days" +%s)

sqlite3 "$DB" <<SQL
CREATE TABLE IF NOT EXISTS licenses(
  license_key TEXT PRIMARY KEY,
  customer_id TEXT NOT NULL,
  hw_fp       TEXT,
  expires_at  INTEGER NOT NULL,
  active      INTEGER NOT NULL DEFAULT 1
);
INSERT OR REPLACE INTO licenses(license_key, customer_id, expires_at, active)
  VALUES('$KEY','$CID',$EXP,1);
SQL
echo "seeded $KEY -> $CID exp=$(date -u -r "$EXP" 2>/dev/null || date -u -d "@$EXP")"
