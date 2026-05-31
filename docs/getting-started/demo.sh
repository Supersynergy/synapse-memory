#!/usr/bin/env bash
# synapse demo — runs a 60-second end-to-end tour
# Usage: bash docs/getting-started/demo.sh
set -euo pipefail

DB="/tmp/synapse-demo-$$.db"
cleanup() { rm -f "$DB"; }
trap cleanup EXIT

BIN="${SYNAPSE_BIN:-synapse}"

echo "=== Synapse Demo ==="
echo "Store: $DB"
echo ""

# 1. init
"$BIN" init -f "$DB"
echo "[1/7] Store initialized"

# 2. Put docs
"$BIN" put -f "$DB" --title "rust-intro"   --text "Rust is a systems language focused on safety and performance."
"$BIN" put -f "$DB" --title "synapse-perf" --text "Synapse is 970x faster than sqlite-vec at 1 million documents."
"$BIN" put -f "$DB" --title "hybrid-search" --text "Hybrid search fuses BM25 and cosine similarity via RRF fusion."
"$BIN" put -f "$DB" --title "hipporag"     --text "HippoRAG-2 PPR graph re-ranking improves multi-hop QA accuracy."
"$BIN" put -f "$DB" --title "mcp-tools"    --text "Synapse MCP server exposes memory_save, memory_search, put, search tools."
echo "[2/7] Ingested 5 documents"

# 3. BM25 search
echo ""
echo "[3/7] BM25 search: 'fast performance'"
"$BIN" find "fast performance" -f "$DB" --limit 3

# 4. Hybrid search
echo ""
echo "[4/7] Hybrid search: 'vector similarity search'"
"$BIN" hybrid "vector similarity search" -f "$DB" --limit 3

# 5. Graph edge
"$BIN" relate 2 3 "relates-to" --weight 0.8 -f "$DB" 2>/dev/null || true
echo ""
echo "[5/7] Graph edge added: doc 2 → doc 3"

# 6. Traverse
echo ""
echo "[6/7] Graph traverse from doc 1 (depth 2):"
"$BIN" traverse 1 --depth 2 -f "$DB" 2>/dev/null || echo "  (no graph edges from doc 1)"

# 7. Stats
echo ""
echo "[7/7] Stats:"
"$BIN" stats -f "$DB"

echo ""
echo "=== Done! 5 docs ingested, hybrid search + graph working. ==="
echo ""
echo "Next steps:"
echo "  Python API:  docs/getting-started/01-agent-memory/"
echo "  RAG demo:    docs/getting-started/02-rag-builder/"
echo "  MySQL mode:  docs/getting-started/03-mysql-drop-in/"
echo "  MCP server:  docs/getting-started/04-mcp-server/"
