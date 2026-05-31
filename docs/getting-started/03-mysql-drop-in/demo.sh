#!/usr/bin/env bash
# 03-mysql-drop-in/demo.sh
# Demonstrates synapsql MySQL wire-protocol compatibility.
# Requires: synapsql binary in PATH (cargo build --release -p synapsql)
set -euo pipefail

DB="./demo.synx"
PORT=3307

cleanup() { kill "$SYNAPSQL_PID" 2>/dev/null || true; rm -f "$DB"; }
trap cleanup EXIT

echo "=== Synapse MySQL drop-in demo ==="

# Remove old db
rm -f "$DB"

# Start synapsql in background
echo "[1] Starting synapsql on port $PORT..."
synapsql --port "$PORT" --db "$DB" &
SYNAPSQL_PID=$!
sleep 1  # wait for bind

# Seed data via mysql client
echo "[2] Seeding data via mysql CLI..."
mysql -h 127.0.0.1 -P "$PORT" -u root --protocol=TCP 2>/dev/null <<'SQL'
CREATE TABLE IF NOT EXISTS docs (id INT AUTO_INCREMENT PRIMARY KEY, title VARCHAR(255), body TEXT);
INSERT INTO docs (title, body) VALUES
  ('Rust async guide',      'Tokio and async/await make Rust async ergonomic.'),
  ('SimSIMD performance',   'AVX-512 dot product 71x faster than scalar loop.'),
  ('Synapse hybrid search', 'BM25 + cosine via RRF fusion, sub 10ms at 100k docs.'),
  ('WordPress on Synapse',  'Drop-in MySQL replacement for WordPress with FTS5.'),
  ('Vector databases 2026', 'Synapse beats Qdrant Pinecone Chroma on embedded use.');
SQL

echo "[3] Full-text search: WHERE body MATCH 'rust'"
mysql -h 127.0.0.1 -P "$PORT" -u root --protocol=TCP 2>/dev/null \
  -e "SELECT id, title FROM docs WHERE body MATCH 'rust';"

echo ""
echo "[4] Regular SELECT"
mysql -h 127.0.0.1 -P "$PORT" -u root --protocol=TCP 2>/dev/null \
  -e "SELECT id, title FROM docs LIMIT 3;"

echo ""
echo "[5] Row count"
mysql -h 127.0.0.1 -P "$PORT" -u root --protocol=TCP 2>/dev/null \
  -e "SELECT COUNT(*) AS total FROM docs;"

echo ""
echo "=== Done. synapsql handles MySQL wire protocol transparently. ==="
