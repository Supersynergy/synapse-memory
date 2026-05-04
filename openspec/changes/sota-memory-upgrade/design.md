# Design — sota-memory-upgrade

## Architecture diff

### Before (v0.3.x)
```
ingest → embed (fastembed/MLX) → docs(FTS5) + vecs(sqlite-vec)
recall → hybrid_search (vec ∪ FTS5) → RRF k=60 → top-k
```
All docs equal-weight. No entity graph. No rerank. Sync extraction = none.

### After (sota)
```
ingest → embed → docs(FTS5) + vecs(sqlite-vec)
       └→ extraction_queue
async worker (RuleExtractor | MlxExtractor)
       → entities + memories(typed, weighted) + memory_edges

recall(params: RecallParams)
  ├── temporal filter (period)
  ├── vec hits      ─┐
  ├── FTS5 hits     ─┤
  ├── entity 1-hop  ─┼─→ rrf_typed(weights) → top-N
  └── heat boost    ─┘                          │
                                                ↓
                                  Reranker (IdentityReranker | OnnxCrossEncoder)
                                                │
                                                ↓
                                            top-k Hits
```

## Mined patterns table
| Pattern | Source repo | Used in | Notes |
|---|---|---|---|
| DBSF (Distribution-Based Score Fusion) as RRF alternative | deepset-ai/haystack `document_joiner.py` | future v2 of `rrf_typed` | ~30 LOC port |
| RRF k=60 industry default | run-llama/llama_index MongoDB hybrid recipe | `rrf_typed` constant | confirmed |
| RRF shape | FlowiseAI/Flowise `RRFRetriever.ts` | `shard.rs::rrf_merge` | shape validated |
| `TextRerank` ms-marco/JINA/bge ONNX | Anush008/fastembed-rs v5 | `synapse-rerank::OnnxCrossEncoder` | ~50 LOC, default model = JINA-rerank-v2-base-multilingual |
| Typed-memory taxonomy + tiered weights | Mem0/Letta/Hindsight (public docs only) | `MemoryType::default_weight()` | Fact 1.20 · Pref 1.15 · Decision 1.10 · Lesson 1.05 · Raw 1.00 · Episodic 0.95 |
| RRF list-of-lists shape `(id, rank, meta)` | vectorize-io/hindsight `tracer.py::add_rrf_merged` | `Store::recall` candidate fusion | shape adapted to Rust tuples, ~10 LOC adapted |
| `parse_date_string("yesterday")` + `Dialect::Us` | stevedonovan/chrono-english (proven via facebook/sapling) | `synapse-temporal::parse_temporal` | wrapper crate, 90 % reuse, 0 LOC of parser logic adapted |

## Model choice rationale: smollm2 1.7B > Phi-4-mini

| Criterion | smollm2-1.7B-Instruct-4bit | Phi-4-mini-4bit | Decision |
|---|---|---|---|
| Size on disk | ~1.0 GB | ~2.2 GB | smollm2 |
| MLX latency / extraction | ~150 ms | ~556 ms | smollm2 |
| JSON-mode reliability | high (Instruct-tuned, 2026 fine-tune) | high | tie |
| Entity-extraction F1 (HF leaderboard reproductions) | ~0.78 | ~0.81 | Phi-4-mini |
| Battery / heat on M4 Max sustained | low | medium | smollm2 |
| Fits on iPhone-class for future mobile sync | yes | borderline | smollm2 |

**Verdict:** smollm2-1.7B default, Phi-4-mini opt-in via `--extractor phi4` flag. Extraction quality A/B will be measured on LongMemEval-S — if Phi-4-mini moves the needle ≥ 2 pt, promote it to default for power users; smollm2 stays for edge/mobile.

QwQ-32B is reserved as fallback when extraction confidence < 0.5 on a doc — invoked only for the long tail.

## Schema (additive, idempotent)
```sql
CREATE TABLE IF NOT EXISTS entities (
  id INTEGER PRIMARY KEY,
  canonical_name TEXT UNIQUE NOT NULL,
  entity_type TEXT NOT NULL,
  alias_json TEXT,
  created_ts INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS memories (
  id INTEGER PRIMARY KEY,
  doc_id INTEGER NOT NULL REFERENCES docs(id),
  memory_type TEXT NOT NULL,
  entity_id INTEGER REFERENCES entities(id),
  weight REAL NOT NULL,
  confidence REAL NOT NULL,
  superseded_by INTEGER REFERENCES memories(id),
  project_tags TEXT,
  created_ts INTEGER NOT NULL,
  updated_ts INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_edges (
  src_id INTEGER NOT NULL REFERENCES memories(id),
  dst_id INTEGER NOT NULL REFERENCES memories(id),
  edge_type TEXT NOT NULL,
  weight REAL NOT NULL,
  created_ts INTEGER NOT NULL,
  PRIMARY KEY (src_id, dst_id, edge_type)
);

CREATE TABLE IF NOT EXISTS extraction_queue (
  doc_id INTEGER PRIMARY KEY REFERENCES docs(id),
  enqueued_ts INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  status TEXT NOT NULL DEFAULT 'pending'
);
```

## Compatibility
- Existing CRDT (`yrs`) untouched.
- Existing Ed25519 signing untouched.
- MCP `synapse_search` / `put` / `find` untouched — `recall()` is a NEW additional API; old hybrid_search remains.
- `sota_migrate()` callable any time, idempotent (`CREATE TABLE IF NOT EXISTS`, `ADD COLUMN ... DEFAULT`).

## Performance budget
| Stage | Target p95 |
|---|---|
| `recall()` end-to-end (top-20, no rerank) | ≤ 8 ms |
| `recall()` with OnnxCrossEncoder rerank top-20 | ≤ 60 ms |
| Async extraction RuleExtractor | ≤ 2 ms / doc |
| Async extraction MlxExtractor (smollm2) | ~150 ms / doc (background, off critical path) |
