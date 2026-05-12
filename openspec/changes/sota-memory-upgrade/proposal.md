# SOTA Memory Upgrade

**Date:** 2026-04-29
**Owner:** Maxim Supersynergy
**Status:** in-progress
**Target:** LongMemEval-S accuracy ≥ 94 % (vs OMEGA 95.4 %, current Synapse hybrid baseline ~76-82 %).

## Why
Synapse already wins iso-recall ANN benchmarks (Sift-1M, 7.9× faster build than faiss-hnsw, parity QPS @ recall 0.99). What it lacks for **agent memory**:

1. No **typed memories** (Fact / Preference / Decision / Lesson / Episodic / Raw). All docs treated equal → ranking blunt.
2. No **cross-encoder rerank**. Pure RRF on FTS5+vec leaves +5-12 pt of LongMemEval headroom on the floor.
3. No **async extraction**. Without entity / fact distillation, recall@k stays bottlenecked by raw chunk similarity.
4. No **temporal grounding** ("yesterday", "last week" don't filter).
5. No **lifecycle** (no decay, no compaction, no supersession).

Mem0, Letta, OMEGA, Hindsight all converged on this stack. Synapse must match parity to be a real competitor in 2026 agent-memory.

## What
Layer additive on existing Store — no breakage of CRDT, Ed25519 signing, or MCP path.

| # | Capability | Crate | LOC |
|---|---|---|---|
| 1 | Typed memory schema + weighted RRF | `synapse-core::sota` | 290 |
| 2 | Cross-encoder rerank (fastembed JINA-rerank-v2 default) | `synapse-rerank` (new) | 115 |
| 3 | Async extraction queue + RuleExtractor + MlxExtractor stub | `synapse-extract` (new) | 220 |
| 4 | LongMemEval-S adapter | `bench/longmemeval/` | 75 |
| 5 | `Store::recall(RecallParams)` fusing vec+FTS+entity+heat | `synapse-core::sota` | pending |
| 6 | Entity 1-hop expansion | `synapse-core::sota` | pending |
| 7 | Temporal parser ("yesterday" → period filter) | `synapse-core::sota` | pending |
| 8 | Lifecycle daemon (evolve/compact/decay) | launchd job | pending |
| 9 | Real LongMemEval-S run (data ~2 GB) | `bench/longmemeval/` | pending |

## Success criteria
1. **LongMemEval-S accuracy ≥ 94 %** on full 500-question set (current baseline TBD on same harness).
2. **No regression** on existing benches: Sift-1M build/QPS, OLTP 7153 ops/s, WP install, MLX 4.6×.
3. **Pure Rust runtime path** — MLX extraction is subprocess, off the critical retrieval path.
4. **Additive migrations only** — `sota_migrate()` is idempotent, drops no columns, breaks no CRDT clients.
5. **Tests:** `cargo test -p synapse-core -p synapse-rerank -p synapse-extract` all green.

## Non-goals
- Replacing existing CRDT/signing/MCP code paths.
- Bumping any dependency to a breaking major version under time budget.
- Re-running ANN benchmarks (already won; not the bottleneck).

## Risks
| Risk | Mitigation |
|---|---|
| smollm2-1.7B extraction quality below Mem0's | RuleExtractor baseline first; A/B vs MLX. QwQ-32B subprocess fallback if needed. |
| Cross-encoder latency on M4 Max | fastembed ONNX, JINA-rerank-v2-base-multilingual, top-k=20 cap. |
| LongMemEval-S data download (~2 GB) blocks CI | Run locally, cache `bench/longmemeval/data/`. |
| Schema drift breaking existing store | `sota_migrate()` idempotent, all `ADD COLUMN ... DEFAULT`. Tested. |
