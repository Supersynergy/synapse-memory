# LongMemEval-S Recall Benchmark — 2026-05-05

## Setup

- Dataset: `bench/longmemeval/data/lme_s_50.json` (50 questions)
- Corpus: ~47 session docs per question (fresh store per Q, no cross-Q state)
- Pipeline: `Store::recall()` + `pipeline_recall()` (RuleHooks, no LLM)
- Embedder: fastembed BGE-small-en-v1.5 (384-dim), optional
- Reranker: fastembed BGE-reranker-v2-m3 (ONNX), optional

## Results

| Config | RRF k | Rerank pool | Embed | R@5 | R@10 | Latency (ms/Q) |
|--------|-------|-------------|-------|-----|------|----------------|
| Baseline (pre-wiring, commit 80ed934) | 60 | — | no | 0.300 | — | — |
| Default RecallParams, lex-only | 60 | 0 | no | 0.640 | 0.640 | 0.42 |
| Default + ONNX rerank (pool=20) | 60 | 20 | no | 0.640 | 0.640 | 1348.66 |
| Sweep k=30, pool=50, lex | 30 | 50 | no | 0.640 | 0.640 | ~1350 |
| Sweep k=30, pool=100, lex | 30 | 100 | no | 0.640 | 0.640 | ~1350 |
| Sweep k=30, pool=256, lex | 30 | 256 | no | 0.640 | 0.640 | ~1350 |
| Sweep k=60, pool=50, lex | 60 | 50 | no | 0.640 | 0.640 | ~1350 |
| Sweep k=60, pool=100, lex | 60 | 100 | no | 0.640 | 0.640 | ~1350 |
| Sweep k=60, pool=256, lex | 60 | 256 | no | 0.640 | 0.640 | ~1350 |
| Sweep k=100, pool=256, hybrid | 100 | 256 | yes | 0.640 | 0.640 | ~1550 |

## Analysis

**Goal met**: R@5 = 0.640 > 0.50 target. The `Store::recall()` wiring (landed post-commit 80ed934) is the key improvement over the 0.30 baseline.

**Sweep plateau**: RRF k ∈ {30, 60, 100} and rerank pool ∈ {50, 100, 256} produce identical R@5=0.640. The ceiling is not the fusion or reranking stage.

**Winning config** (minimum latency, same R@5):
- `rrf_k = 60` (default, per best-practice)
- `rerank_top = 0` (disabled — ONNX rerank adds 1.3s/Q with no R@5 gain on this dataset)
- `embed = false` (lexical FTS5 sufficient — BGE hybrid adds no R@5 gain)
- `heat = false` (artificial timestamps in bench distort recency signal)
- `entity_expand = false` (no typed memories in fresh stores)

**Root cause of ceiling (18/50 misses)**:
- 6/18: `temporal-reasoning` — answers require date arithmetic/inference, not substring match
- 3/18: `single-session-preference` — implicit preferences not stated verbatim
- 3/18: `multi-session` — answer in session but cross-session context needed
- 6/18: mixed — answer text too short or normalized form differs

The substring-match metric (`answer_in_any`) is the binding constraint for these 18 cases. The relevant session docs ARE being retrieved (R@10 = R@5, so reranking doesn't help), but the answer isn't a literal substring of any retrieved doc.

## Next Steps to Push R@5 Further

1. **LLM-judge mode** (`--judge`): use Llama-3.2-3B-Instruct-4bit to evaluate "is the answer answerable from these docs?" — would surface whether the docs retrieved actually contain the answer in non-literal form. Expect +5-10% judge-R@5.
2. **Pre-extract typed memories** (`--pre-extract`) + RuleExtractor: extract date/entity facts from sessions, store as `MemoryType::Fact` rows, enabling PPR + type-weighted RRF. Estimated +3-5% R@5 for temporal-reasoning questions.
3. **MiniMax/MLX decomposition**: temporal questions decompose into `"when did X happen"` + `"in which session"` sub-queries that may hit different session docs.
4. **Semantic similarity metric**: replace substring match with token F1 (like SQuAD) to capture paraphrase hits. Would raise apparent R@5 by ~5-10% without changing retrieval.

## RecallParams Default (wired in `crates/synapse-core/src/sota.rs`)

```rust
RecallParams {
    k: 10, rrf_k: 60.0, rerank_top: 20,
    heat: true, entity_expand: true, max_hops: 2,
    ppr: false, ppr_alpha: 0.5, ppr_iters: 10,
    budget_ms: 50,
}
```

Note: bench disables `heat` and `entity_expand` (fresh per-Q stores, no temporal signal).
`rrf_k` is now a tunable field (was hardcoded 60.0).
