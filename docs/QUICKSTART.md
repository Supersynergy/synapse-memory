# Synapse in 5 minutes

Single-file embedded AI-memory for agents. SQLite-extension. WordPress-compatible. 970× faster than sqlite-vec @ 1M. HippoRAG-2 graph native.

## Install

```bash
cargo install synapse-cli
# OR build from source:
git clone https://github.com/Supersynergy/synapse.git
cd synapse && cargo build --release -p synapse-cli
```

## 60-second tour

```bash
# 1. Init store
synapse init -f brain.db

# 2. Ingest docs
echo "HippoRAG-2 outperforms GraphRAG on multi-hop QA" | synapse put -f brain.db --title "hipporag"
echo "Synapse uses SimSIMD for vec search, 71× peak speedup"  | synapse put -f brain.db --title "simsimd"

# 3. Search (hybrid FTS5+vec+BM25, 8ms)
synapse hybrid "graph retrieval"

# 4. Add edges (graph layer)
synapse graph relate 1 2 references --weight 0.9
synapse graph traverse 1 --depth 2

# 5. One-shot grounding (hybrid → PPR → traverse → JSON)
synapse ground "graph retrieval" --k 10 --depth 2
```

## What you get out-of-the-box

- **Hybrid retrieval** (FTS5 + vec + BM25) — `synapse hybrid` 8ms
- **Vec search** (SimSIMD, f16/i8/binary cascade) — 970× vs sqlite-vec @ 1M
- **Graph layer** (PageRank, communities, traverse, shortest-path, Cypher) — `synapse graph *`
- **HippoRAG-2 PPR** (HippoRAG-2 §3.2 native Rust) — `synapse graph ppr '{"42":1.0}'`
- **One-shot grounding** (hybrid + PPR + traverse → JSON bundle) — `synapse ground`
- **Auto-relate** — extractor emits triples, edges populate automatically
- **MCP server** (`synapse-server`) — drop-in for Claude/agents
- **WP/MySQL/PG protocol** (`synapsql`) — WordPress plugin works on Synapse without code change

## Server mode

```bash
synapse-server --graph-db brain.db --port 8080
# HTTP endpoints:
curl localhost:8080/graph/pagerank
curl localhost:8080/graph/communities
curl localhost:8080/graph/neighbors/42/10
curl -X POST localhost:8080/graph/cypher -d '{"q":"MATCH (n)-[:r]->(m) RETURN n"}'
# SSE live edges:
curl localhost:8080/graph/live
```

## Python SDK

```python
from synapsql import connect  # sdk/python/synapsql/
db = connect("brain.db")
db.put("Memory text")
db.hybrid("query", k=10)
db.ground("query", depth=2)
db.graph.relate(1, 2, "references", weight=0.9)
db.graph.ppr({1: 1.0}, alpha=0.5)
```

## When to use Synapse

✅ Agent memory (Claude Code, Cursor, custom CLIs)
✅ Embedded RAG (single-file deployment)
✅ WP-site search/recommend (drop-in MySQL replacement)
✅ Multi-hop QA on local corpus
✅ Vec-DB for laptop-scale data (≤10M docs)

## When NOT

❌ >100M-row analytics (use DuckDB/ClickHouse)
❌ Distributed multi-region writes (use CockroachDB)
❌ Heavy multi-tenant SaaS auth (use Postgres + auth-service)

## Next steps

- [Why Synapse](WHY-SYNAPSE.md) — positioning, benchmarks, moat
- [Grounding stack](grounding.md) — `synapse ground` deep-dive
- [TRUTH-2026-05-10](TRUTH-2026-05-10.md) — verified numbers, single source of truth
- `eval/usecases/UC58_grounding_race.py` — race-bench harness for your own corpus
