# Synapse SOTA Progress — 2026-04-29

## Status by phase
| Phase | Status |
|---|---|
| 0 — Mining | DONE — `docs/MINING-SOTA-2026-04-29.md` |
| 1 — ThinkRich roadmap | DONE — `docs/SOTA-ROADMAP-2026-04-29.md` |
| 2 — Tier-1 implementation | PARTIAL — typed schema + rerank crate + extract crate compiling, all tests green |
| 3 — Bench harness | SKELETON — `bench/longmemeval/longmemeval_adapter.rs` |
| 4 — Report | this file |

## Mining — top patterns (5)
1. **Haystack `document_joiner.py`** (deepset-ai) — DBSF (Distribution-Based Score Fusion) as RRF alternative. Port: ~30 LOC.
2. **MongoDB hybrid recipe** (run-llama/llama_index) — confirms `k=60` industry default for RRF.
3. **Flowise RRFRetriever.ts** (FlowiseAI) — confirms our `shard.rs::rrf_merge` shape is correct.
4. **fastembed v5 `TextRerank`** — already a workspace dep; ms-marco / JINA / bge rerankers via ONNX. Used directly in `synapse-rerank` (~50 LOC).
5. **Mem0/Letta/Hindsight** — public docs only (repos private/empty in grep.app). Synthesized typed-memory taxonomy + tiered weights into `MemoryType::default_weight()`.

Repos cited: `deepset-ai/haystack`, `run-llama/llama_index`, `FlowiseAI/Flowise`, `Anush008/fastembed-rs`.

Misses: OMEGA repo (paper-only), Hindsight (private), explicit Rust ms-marco impls (none on grep.app — fastembed is the canonical path).

## What got implemented
| File | LOC | Purpose |
|---|---|---|
| `crates/synapse-core/src/sota.rs` | 290 | `MemoryType`, `Memory`, `MemoryEdge`, `RecallParams`, `sota_migrate`, `put_memory`, `supersede`, `enqueue_extraction`, `pop_extraction_batch`, `rrf_typed`. 5 tests. |
| `crates/synapse-core/src/lib.rs` | +5 | Re-export `sota` module. |
| `crates/synapse-rerank/Cargo.toml` | 24 | New crate, `onnx` feature gates fastembed. |
| `crates/synapse-rerank/src/lib.rs` | 115 | `Reranker` trait, `IdentityReranker`, `OnnxCrossEncoder` (JINA-rerank-v2-base-multilingual default). 2 tests. |
| `crates/synapse-extract/Cargo.toml` | 19 | New crate, `mlx` feature for smollm2 subprocess. |
| `crates/synapse-extract/src/lib.rs` | 220 | `Extractor` trait, `RuleExtractor`, `MlxExtractor` stub, `upsert_entity`, `run_once`, `ingest_and_extract`. 4 tests. |
| `bench/longmemeval/longmemeval_adapter.rs` | 75 | LongMemEval-S JSONL parser + recall metric. 2 tests. |
| `Cargo.toml` (workspace) | +4 | Added 2 members. |
| `docs/MINING-SOTA-2026-04-29.md` | full | Mining report. |
| `docs/SOTA-ROADMAP-2026-04-29.md` | full | ThinkRich matrix + arch diff + risk register. |

**Total new code:** ~750 LOC. **Build status:** `cargo check -p synapse-core -p synapse-rerank -p synapse-extract` clean. **Tests:** 6 new (rerank+extract) + 5 new (sota) = **11 / 11 passing**.

## Schema added (additive, idempotent)
```sql
entities(id, canonical_name UNIQUE, entity_type, alias_json, created_ts)
memories(id, doc_id→docs, memory_type, entity_id→entities, weight, confidence,
         superseded_by→memories, project_tags, created_ts, updated_ts)
memory_edges(src_id, dst_id, edge_type, weight, created_ts)  -- composite PK
extraction_queue(doc_id PK, enqueued_ts, attempts, last_error, status)
```
Default weights tuned per OMEGA-style taxonomy: Fact 1.20 · Preference 1.15 · Decision 1.10 · Lesson 1.05 · Raw 1.00 · Episodic 0.95.

## What's left (Tier-2)
| # | Item | Effort |
|---|---|---|
| T1.5 | Wire `recall()` in `synapse-core` fusing vec+FTS+entity+heat with `rrf_typed` + reranker hook | 0.5d |
| T1.4 | Entity 1-hop expansion in recall pipeline | 0.5d |
| T1.3 | MLX subprocess wiring (real smollm2-1.7B-Instruct-4bit invocation) | 1d |
| T2.6 | Temporal parser ("yesterday", "last week") + period filter | 2d |
| T2.7 | Lifecycle daemon (evolve/compact/decay) via launchd job | 3d |
| T2.8 | MemFS git mirror on `synapse-wal` | 4d |
| T2.9 | Run real LongMemEval-S after data download (~2GB) | 2d |
| - | Migrate existing `Store::open` to call `sota_migrate` automatically | 0.25d |

## Expected LongMemEval delta
Per ThinkRich estimates in roadmap:
- Cross-encoder rerank: **+5 to +12pt**
- Typed memory + weights: **+2 to +4pt**
- Multi-signal RRF: **+2 to +3pt**
- Entity 1-hop: **+1 to +3pt**
- Async extraction (data quality): unlocks the above

**Combined upside: +10 to +18pt** vs current Synapse hybrid baseline → competitive with OMEGA 95.4% if extraction quality matches Mem0 (smollm2-1.7B is borderline; QwQ-32B fallback for scoring runs).

## Next-step recommendation
Wire `Store::recall(params: RecallParams) -> Vec<Hit>` in `synapse-core::sota` (next 4 hours). Once `recall()` exists, every Tier-2 item plugs in cleanly without further refactor. Defer real MLX subprocess until LongMemEval-S baseline number (with `RuleExtractor`) is on the board — gives a clean isolated A/B for the extractor swap.

## Constraint adherence
- Pure Rust — yes (MLX is subprocess, off critical path).
- Additive only — yes (no migrations drop columns; no breakage of CRDT/Ed25519/MCP).
- RTK wrappers — yes (`rtk cargo check`).
- No npm — n/a.
- Mining-first — yes (3 reusable patterns identified, fastembed reused, RRF kept).
