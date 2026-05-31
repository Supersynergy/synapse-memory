# Synapse Quickstart

Single-file embedded AI memory. No cloud. No daemon required.
970× faster than sqlite-vec @ 1M docs. HippoRAG-2 graph native.

---

## 60-second install

```bash
# Option A: cargo (Rust toolchain required)
cargo install synapse-cli

# Option B: build from source
git clone https://github.com/Supersynergy/synapse.git
cd synapse && cargo build --release -p synapse-cli
cp target/release/synapse ~/.local/bin/

# Option C: Python (PyO3 wheels via maturin)
maturin develop -p synapse-py --release   # from repo root

# Option D: Docker
docker run --rm -v $(pwd):/data ghcr.io/supersynergy/synapse:latest \
  synapse -f /data/brain.db init
```

---

## 5-minute first app

```bash
# 1. Create a store
synapse init -f brain.db

# 2. Add documents
synapse put -f brain.db --title "intro" --text "Synapse is a fast embedded vector DB"
synapse put -f brain.db --title "perf"  --text "970x faster than sqlite-vec at 1M docs"
synapse put -f brain.db --title "rust"  --text "Built in Rust with SimSIMD kernels"

# 3. Keyword search (BM25)
synapse find "vector" -f brain.db

# 4. Hybrid search (BM25 + vector, recommended)
synapse hybrid "fast embedded" -f brain.db

# 5. Graph: add edge, traverse
synapse relate 1 2 references --weight 0.9 -f brain.db
synapse traverse 1 --depth 2 -f brain.db

# 6. One-shot grounding (hybrid + PPR → JSON bundle for LLM)
synapse ground "fast embedded" -f brain.db --k 10 --depth 2

# 7. Stats
synapse stats -f brain.db
```

---

## Fast local agent mode

For Claude Code, Cursor, Codex, and local agents, run the daemon once and use
`synx-fast` for socket recall. This avoids one process/socket setup per query
and keeps project memories scoped.

```bash
# build local fast path
cargo build --release -p synapsed --bin synapsed --bin synx-fast

# start daemon
synapsed --file ~/.synapse/brain.db --sock /tmp/synapse.sock

# health check
SYNAPSE_SOCK=/tmp/synapse.sock synx-fast doctor

# scoped memory
synx-fast put --scope my-project --title "decision" \
  "Decision: use daemon-native scoped recall for agent context"

# compact context block for an LLM prompt
synx-fast context --scope my-project "scoped recall agent context" --budget 600

# keepalive batch for hooks
printf 'scoped recall\nagent context\n' |
  synx-fast batch hybrid --scope my-project --limit 8
```

One-command smoke test from this repo:

```bash
make smoke-fast
```

Useful hook env vars:

| Env | Default | Purpose |
|-----|---------|---------|
| `SYNX_FAST_BIN` | `synx-fast` | recall and batch CLI |
| `SYNX_FRESH_BIN` | `synx` | latest-doc/version guard CLI |
| `SYNAPSE_SOCK` | `/tmp/synapse.sock` | daemon socket |
| `SYNAPSE_SCOPE_KEY` | `scope` | metadata key for scoped recall |
| `TELEPATHY_SCOPE` | cwd basename | force one Telepathy write scope |

---

## Common patterns (recipes)

### Agent memory (Python)

```python
from synapse_rs import Synapse

s = Synapse("./memory.db")
s.put("turn-1", "user said hello", metadata={"session": "abc"})
s.put("turn-2", "response: hi there!", metadata={"session": "abc"})
results = s.search("greetings", k=3)
# → [(id, text, score), ...]
s.close()
```

Full example: `docs/getting-started/01-agent-memory/`

---

### Hybrid RAG (Python, with embeddings)

```python
from synapse_rs import Brain
from sentence_transformers import SentenceTransformer

model = SentenceTransformer("all-MiniLM-L6-v2")
brain = Brain("rag.db")

# Ingest with embedding
text = "Synapse uses SimSIMD for 71x speedup"
emb = model.encode(text, normalize_embeddings=True).tolist()
brain.put_with_embedding(text, emb)

# Retrieve
q_emb = model.encode("fast vector search", normalize_embeddings=True).tolist()
hits = brain.search_hybrid("fast vector", q_emb, limit=5)
```

Full example: `docs/getting-started/02-rag-builder/`

---

### MySQL drop-in (WordPress)

```bash
# Start synapsql on port 3307 (avoids conflict with stock MySQL)
synapsql --port 3307 --db ./brain.db

# Any MySQL client connects — no code changes
mysql -h 127.0.0.1 -P 3307 -u root \
  -e "SELECT id, title FROM docs WHERE body MATCH 'rust async'"

# WordPress wp-config.php:
# define('DB_HOST', '127.0.0.1:3307');
```

Full example: `docs/getting-started/03-mysql-drop-in/`

---

### MCP server (Claude Code / Cursor)

```bash
# Build and start
cargo build --release -p synapse-mcp
synapse-mcp --sock /tmp/synapse.sock --db ~/.claude/brain.db

# Wire into ~/.claude/mcp.json:
{
  "mcpServers": {
    "synapse": {
      "command": "synapse-mcp",
      "args": ["--sock", "/tmp/synapse.sock", "--db", "/Users/you/.claude/brain.db"]
    }
  }
}
```

Tools: `memory_save` `memory_search` `memory_recent` `put` `search` `timeline` `synapse_merge` `synapse_verify`

Full example: `docs/getting-started/04-mcp-server/`

---

### Migrate from Pinecone / Chroma

```bash
# Export existing data to JSONL: {"id":"doc-1","text":"...","embedding":[0.1,...]}
synapse put-batch --file export.jsonl -f brain.db
synapse hybrid "your query" -f brain.db
```

---

## CLI reference

```
synapse [OPTIONS] <COMMAND>

OPTIONS:
  -f, --file <FILE>    Store path [default: .synapse/brain.db]

SEARCH:
  find   <query>       BM25 full-text search
  vec    <query>       Vector kNN (requires embeddings in store)
  hybrid <query>       Hybrid BM25+vec+RRF  ← use this
  ground <query>       hybrid + PPR graph re-rank → JSON bundle

WRITE:
  init                 Create new store
  put                  Append document (--text or stdin)
  merge  <snap>        Merge remote snapshot (CRDT)

GRAPH:
  relate <a> <b> <rel> Add directed edge
  traverse <id>        Walk from node
  ppr  <seed-json>     HippoRAG-2 Personalized PageRank
  pagerank             Full-graph PageRank scores
  communities          Detect communities (Louvain)

OPERATIONS:
  stats                Doc count, store size, index status
  snap                 Export .brainpack snapshot
  backup <dest>        SQLite hot backup
  federate             Join/start federation cluster
  shard                Shard store across files
  learn <id> <score>   Record retrieval feedback
  calibrate            Rebuild vec index + tune EF params
  keygen               Generate Ed25519 signing keypair
```

---

## Doctor / health check

```bash
synapse stats -f brain.db
# docs: 42  store_size: 1.2MB  vec_index: f16  graph_edges: 18

synapse drift-check -f brain.db
# checks index/store consistency
```

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `store open: unable to open database` | `mkdir -p $(dirname brain.db)` then retry |
| Vector search returns nothing | Use `put_with_embedding()` — `put` alone stores text only |
| `synapsql: port 3306 in use` | Use `--port 3307`, set `DB_HOST=127.0.0.1:3307` in app |
| `maturin develop` fails | `pip install maturin && rustup update stable` |
| Slow hybrid search (>50ms) | Run `synapse calibrate -f brain.db` to rebuild index |
| `synapse-mcp` not found in PATH | `cargo install --path crates/synapse-mcp` |
| MCP socket not found | Ensure daemon is running: `ps aux | grep synapse-mcp` |

---

## Next steps

| Goal | Guide |
|------|-------|
| Agent memory (Mem0 replacement) | `docs/getting-started/01-agent-memory/` |
| RAG pipeline (Pinecone replacement) | `docs/getting-started/02-rag-builder/` |
| WordPress / MySQL drop-in | `docs/getting-started/03-mysql-drop-in/` |
| Claude Code / Cursor via MCP | `docs/getting-started/04-mcp-server/` |
| Image + text multimodal search | `docs/getting-started/05-multimodal-search/` |
| LangChain adapter | `integrations/langchain/` |
| LlamaIndex adapter | `integrations/llamaindex/` |
| Architecture | `ARCHITECTURE.md` |
| Benchmarks vs Qdrant/Pinecone/Chroma | `BENCH_2026-05-10.md` |
