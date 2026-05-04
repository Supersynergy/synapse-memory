# Synapse SOTA Roadmap — 2026-04-29

Target: beat OMEGA (95.4% LongMemEval), Hindsight (91.4%), Mem0, Letta on retrieval quality + latency, while keeping CRDT/Ed25519/MCP moat.

## ThinkRich prioritization matrix

Score = `Impact (1-10) × Moat-multiplier ÷ Effort (days)`. Moat-multiplier: 1.0 catch-up, 1.5 differentiator, 2.0 unique.

| # | Feature | Impact | Effort (d) | Moat × | Score | Tier |
|---|---|---|---|---|---|---|
| 1 | **Cross-encoder rerank** (fastembed ms-marco-MiniLM-L-6-v2 ONNX, top-K=20) | 9 | 1.5 | 1.0 | 6.0 | T1 |
| 2 | **Typed memory** (memory_type, entity_id, weight, superseded_by) | 8 | 1.0 | 1.5 | 12.0 | T1 |
| 3 | **Async LLM extraction queue** (smollm2-mlx subprocess → facts/entities) | 8 | 2.5 | 1.5 | 4.8 | T1 |
| 4 | **Entity graph** (memory_edges + 1-hop expansion in recall) | 7 | 1.5 | 1.5 | 7.0 | T1 |
| 5 | **Multi-signal RRF** (vec+FTS+entity+type-weighted+heat) into single `recall()` | 9 | 1.0 | 1.0 | 9.0 | T1 |
| 6 | Temporal parser ("last week", "yesterday") + period filter | 6 | 2.0 | 1.0 | 3.0 | T2 |
| 7 | Lifecycle: evolve/compact/decay/supersede via launchd job | 7 | 3.0 | 1.5 | 3.5 | T2 |
| 8 | MemFS git mirror (Letta-style journaling on synapse-wal) | 5 | 4.0 | 2.0 | 2.5 | T2 |
| 9 | LongMemEval bench harness + nightly run | 9 | 2.0 | 1.0 | 4.5 | T2 |
| 10 | DBSF fusion mode (Haystack pattern) | 4 | 0.5 | 1.0 | 8.0 | T1.5 |

## Top-5 features that move the benchmark most
1. **Cross-encoder rerank** — empirically +5-12pt on LongMemEval-style tasks
2. **Typed memory + type weights** — +2-4pt; cheap; foundation for everything else
3. **Multi-signal RRF** — +2-3pt; ties existing primitives together
4. **Entity graph 1-hop** — +1-3pt on multi-hop questions
5. **Async extraction** — enables 2/4 to actually have data; quality depends on extractor LLM

Expected combined delta: **+10 to +18pt** vs current Synapse baseline → competitive with OMEGA's 95.4%.

## Architecture diff (current → target)

```
                    CURRENT                         TARGET
                    -------                         ------
ingest:  put(text) → embed → docs+vec      put(text) → embed → docs+vec
                                          + enqueue extraction job
                                          → (async) smollm2 → memories(typed) + edges

recall:  search(q) → vec+FTS RRF → heat    recall(q,k) →
                                            vec(top-200) +
                                            FTS(top-200) +
                                            entity-1hop(top-50) +
                                            type-weighted RRF +
                                            heat decay +
                                            cross-encoder rerank(top-20) →
                                            top-K
```

### File-level changes
| File | Change | LOC |
|---|---|---|
| `crates/synapse-core/src/db.rs` | + `memories` migration, + `memory_edges`, + columns | +120 |
| `crates/synapse-core/src/types.rs` | + `MemoryType`, `MemoryEdge`, `RecallParams` | +60 |
| `crates/synapse-core/src/lib.rs` | re-export new types | +5 |
| **NEW** `crates/synapse-core/src/recall.rs` | unified `recall()` fusing all signals | +200 |
| **NEW** `crates/synapse-rerank/` | cross-encoder via fastembed TextRerank | +180 |
| **NEW** `crates/synapse-extract/` | tokio queue + Extractor trait + smollm2 backend | +250 |
| `crates/synapse-learn/src/heat.rs` | reuse as-is | 0 |
| `crates/synapse-learn/src/db.rs` | + `learn_type_weight` table | +20 |
| `Cargo.toml` (workspace) | + 2 members | +2 |
| **NEW** `bench/longmemeval_adapter.rs` | LongMemEval-S loader stub | +120 |

## Risk register
| Risk | Mitigation |
|---|---|
| fastembed rerank API churn | pin v5; feature-gate behind `rerank` |
| ORT model cold-start (~200ms first call) | warmup on Store::open(); cache session in `OnceCell` |
| smollm2 MLX subprocess latency on M-series | run async via tokio queue, never block ingest path |
| Schema migration breaks existing DBs | additive only; ALTER TABLE ADD COLUMN; never drop |
| Cross-encoder breaks <1ms cached recall claim | rerank is opt-in via `RecallParams.rerank=true`; default off for cached |
| LongMemEval data licensing | use LongMemEval-S (public) for nightly; flag M/L if private |
| Breaking CRDT/Ed25519/MCP | additive migrations; sign new memory rows same as docs |

## Sequencing (next 5 working days)
- **D1**: typed schema migration + RecallParams type (T1.2) — 1d
- **D2 AM**: multi-signal `recall()` (T1.5) — 0.5d
- **D2 PM**: cross-encoder crate scaffold + fastembed adapter (T1.1) — 0.5d
- **D3**: extraction crate scaffold + smollm2 subprocess (T1.3) — 1d
- **D4**: entity-graph 1-hop expansion (T1.4) + DBSF mode (T1.10) — 1d
- **D5**: LongMemEval adapter + first run (T2.9) — 1d

## Out-of-scope (intentionally)
- New ANN backend (have synapse-ultra/HNSW)
- New embedder (fastembed + MLX cover it)
- MemFS git mirror (T2.8 — defer until benchmark validates extraction quality)
