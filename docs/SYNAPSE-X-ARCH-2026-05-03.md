# Synapse-X — Krass-Architektur (2026-05-03)

Ziel: SOTA agent-memory. Beat OMEGA 95.4% LongMemEval-S. Sub-50ms recall p95.

## Kern-Insight

Bisherige systeme (OMEGA, Mem0-v3, Hindsight, Supermemory) sind Python-monolithen
mit blocking LLM-calls auf hot path. Synapse-X dreht das um: **alles teure passiert
beim INGEST, recall ist nur fusion + rerank**. Agentic layer (decompose/HyDE/grade)
ist **opt-in** — feuert nur wenn query es braucht (nicht jede frage).

## Pipeline

```
INGEST (async, parallelisiert via tokio + rayon):
  text
   ├─ chunks (proposition-level, NEW crate `synapse-chunk`)
   ├─ embed BGE-small-384 (MLX Metal, batch 32 → 0.22ms/doc)
   ├─ ColBERT late-interaction tokens (next-plaid-onnx, 128-dim/tok)  [NEW]
   └─ Mem0-v3 hierarchical extract (MiniMax-M2 1 call → facts+summary+topics+event_date)
        ├─ entities (gazetteer-NER + canonical merge)
        └─ edges (supports/contradicts/about — auto-induced)

WRITE: docs · vec(sqlite-vec) · colbert(NEW) · memories · entities · memory_edges
       · FTS5(text) · FTS5(summary) · FTS5(topics) · idx(event_date)

RECALL (5 parallel signals, NO LLM in hot path):
  query
   ├─ vec dense top-50           (simsimd Metal-dispatch, <2ms)
   ├─ FTS5 BM25 top-50           (<1ms)
   ├─ ColBERT MaxSim top-50      (PLAID, <5ms)              [NEW]
   ├─ PPR seeded by ↑3 fused      (HippoRAG-2, alpha 0.5, <5ms)
   └─ event_date period-filter   (only when temporal cue)   [NEW]
                ↓
       RRF type-weighted fusion (Fact 1.20 · Pref 1.15 · Decision 1.10 · …)
                ↓
       CASCADE rerank
         ├─ stage-1: BGE-reranker-v2-m3 (50→20, ~1ms/pair)
         └─ stage-2: Qwen3-Reranker-4B  (20→k, ~5ms/pair)
                ↓
                top-k

AGENTIC LAYER (opt-in, fire only when needed):
  • decompose if cue regex hits (" and "/" or "/temporal markers)
  • HyDE rescue if total hits < threshold
  • Self-RAG grade if confidence < grade_floor (relevance_floor > 0)
  • multi-pass execution

LIFECYCLE (background daemons, never block recall):
  • synapse-extract-worker — async LLM extract on queue
  • synapse-lifecycle nightly — evolve+compact+heat-decay
  • synapse-learn         — CatBoost type-weight tuning on click-through
```

## Status (was schon live ist)

| Layer | Status | Crate | Tests |
|---|---|---|---|
| Schema (typed memories + edges + entities + queue + event_date) | ✅ live | core/sota.rs | 13 |
| MLX BGE-small embedder | ✅ default | synapse-core feat=`embed-mlx` | parity 1.0 vs fastembed |
| Hybrid recall (vec+FTS RRF) | ✅ live | core/db.rs | 41 (existing) |
| **PPR HippoRAG-2** | ✅ NEW | core/ppr.rs | 5 |
| Entity 1-hop BFS expansion | ✅ live | core/sota.rs `multi_hop_neighbors` | n/a |
| Type-weighted RRF | ✅ live | core/sota.rs `rrf_typed` | 1 |
| Heat decay (97%/day) | ✅ live | core/sota.rs Store::recall | n/a |
| Pipeline (decompose+HyDE+SelfRAG+RRF) | ✅ live | core/sota_pipeline.rs | 4 |
| LLM hooks (MiniMax-M2) | ✅ NEW | extract/minimax.rs | 1 |
| Mem0-v3 hierarchical extract | ✅ NEW | extract/minimax.rs | n/a |
| Cascade rerank skeleton | ✅ NEW | rerank/cascade.rs | 2 |
| BGE-reranker-v2-m3 ONNX | ✅ default | rerank fastembed onnx | 2 |
| Lifecycle daemons + launchd | ✅ NEW | extract/bin/{worker,lifecycle}.rs | n/a |
| Async extract queue | ✅ live | core/sota.rs + extract::run_once | 1 |
| evolve_on_ingest + compact | ✅ live | core/sota_pipeline.rs | 2 |

## Was noch FEHLT für 95%+ R@5

| # | Feature | Effort | Erwartet +pt |
|---|---|---|---|
| A | **ColBERT late-interaction** (next-plaid-onnx 3rd retrieval signal) | 10h | +3-5 |
| B | **Propositional chunking** (`synapse-chunk` neu, Dense-X pattern) | 6h | +2-4 |
| C | **Bench mit pre-extract** (worker läuft VOR recall — typed memories aktiv) | 2h | unlock +6-15 |
| D | **PPR im bench-flag** (`--ppr` aktivieren) | 0.5h | +5-8 |
| E | **Cascade qwen3-reranker-4b stage-2** (currently nur stage-1 bge-m3) | 4h | +2-3 |
| F | **Temporal cue parser** (event_date period filter aktivieren) | 3h | +1-3 |
| G | **AgeMem RL tool-policy** (long-term moat) | 24h | +4-7 |

Realistic mit **C+D+A+E+B** in 22h: erwartet **64% → 87-92% R@5**. Mit AgeMem rl
on top zusätzlich +4-7pt → **91-99% R@5**.

## Hot-Path Latency-Budget (target p95 < 50ms)

```
vec top-50           2ms
FTS5 top-50          1ms
ColBERT top-50       5ms
PPR (cached graph)   5ms
event-filter         1ms
RRF-fuse            <1ms
BGE-rerank 50→20    20ms (20 pairs × 1ms)
Qwen3-rerank 20→k   25ms (5 pairs × 5ms parallel batch)
─────────────────────────
Total p95:          ~50-60ms
```

Agentic layer (opt-in) addiert MiniMax round-trip = ~1-2s wenn fired, sonst 0.

## Production-Deploy

Single binary release:
```
synapsed serve            → MCP server :9477 (existing)
synapse-extract-worker    → continuous LLM extract
synapse-lifecycle         → nightly via launchd
synx hybrid "<q>" 20       → CLI hot path
```

Datenpfad: `~/.synapse/brain.db` (WAL mode, 256MB mmap).
LLM: MINIMAX_API_KEY (extract+agentic), Ollama-fallback wenn missing.

## Bench-Plan (nächster schritt)

1. Bench-flag `--ppr` (params.ppr=true)
2. Bench-flag `--pre-extract` (run minimax-extract synchronously before recall)
3. Cache reranker-model `~/.cache/huggingface/hub/Xenova--bge-reranker-v2-m3/`
4. Run full 50 mit: `--embed --ppr --pre-extract --rerank-top 20 --use-minimax`
5. A/B ablation: jedes feature off einzeln → impact-attribution
6. Wenn 95%+ erreicht: ship.
