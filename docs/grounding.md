# Synapse grounding stack

Synapse ships a HippoRAG-2 grounding pipeline natively. No Neo4j, no GraphRAG/Microsoft stack, no Cypher engine. Everything composes existing in-tree crates.

## CLI surface

```bash
# One-shot grounding bundle (hybrid → PPR → traverse → JSON)
synx ground "<query>" [--k 20] [--depth 2] [--alpha 0.5] [--iters 10]

# Graph subcommand-tree
synx graph relate FROM TO REL [--weight W]
synx graph pagerank [--n 20] [--damping 0.85] [--iters 20]
synx graph ppr SEEDS_JSON [--alpha 0.5] [--iters 10] [--limit 30]
synx graph communities [--max-iters 20] [--top-n 20]
synx graph neighbors NODE_ID [--top-k 50] [--rel REL]
synx graph traverse START_ID [--depth 3] [--top-k-per-hop 10] [--decay 0.7]
synx graph path FROM TO [--max-depth 5]
synx graph count
```

## HTTP surface (server)

| Endpoint | Method |
|---|---|
| `/graph/health` | GET |
| `/graph/neighbors/{id}/{k}` | GET |
| `/graph/pagerank` | GET |
| `/graph/communities` | GET |
| `/graph/path/{from}/{to}/{depth}` | GET |
| `/graph/cypher` | POST |
| `/graph/live` | SSE |

## Pipeline

```
ingest
  → docs.put
  → enqueue_extraction(doc_id)
extract worker
  → Extractor::extract(text)
    → Vec<ExtractedMemory { fact, entity, relations, .. }>
  → upsert_entity(s) for each (s, v, o)
  → relate_extracted() writes edges (auto-relate hook)
  → put_memory(typed) for each fact

retrieve
  hybrid    : FTS5 + vec + BM25                           ~8ms
  ppr       : seeds → memory_edges PPR (HippoRAG-2 §3.2)  <5ms / 1k seeds
  traverse  : Dijkstra-style w/ score-decay + top-k cap   <5ms
  ground    : hybrid + ppr + traverse fused, JSON bundle  ~20-60ms
```

## Why no GraphRAG/Neo4j

| Capability | Synapse | Microsoft GraphRAG |
|---|---|---|
| Setup | 0 — single binary | Docker + JVM + Cypher + indexer |
| Vec search | SimSIMD 0.06ms | plugin 50ms |
| PPR / multi-hop | `ppr.rs` HippoRAG-2 | community-summarize at $$$ |
| Cypher | `/graph/cypher` POST | native |
| CSR cache | yes (10-100× SQL CTE) | no |
| Async entity extract | `synapse-extract` worker (smollm2 / mlx) | GPT-4 hot-path |
| End-to-end / query | 20-60ms / $0 | 30s / $0.50 |

HippoRAG-2 (Gutiérrez et al. 2026) demonstrates parity-or-better on multi-hop benchmarks (MuSiQue, 2WikiMQA, HotpotQA) at fraction of cost. Synapse's `ppr.rs` ports the algorithm into a `rusqlite::Connection`-native form so it composes with `Store::recall` without lifting the graph into memory.

## Eval

`eval/usecases/UC58_grounding_race.py` — race-bench for hybrid / ppr / traverse / ground over `eval/golden/grounding.jsonl`. Reports mean / p50 / p95 latency + recall@K per strategy.

```bash
python3 eval/usecases/UC58_grounding_race.py --db .synapse/brain.db --k 10
```

## When to extend

- Add new relation extractor (LLM): implement `Extractor::extract` returning `ExtractedMemory.relations: Vec<ExtractedRelation>`. Auto-relate writes edges automatically.
- Add new traversal algorithm: extend `synapse_graph::algorithms` and expose via CLI in `crates/synapse-cli/src/main.rs` `GraphCmd` enum.
- Add new strategy to UC58 race-bench: drop a `s_<name>(db, q, k)` function and register in `STRATEGIES`.
