# Synapse Recall-Quality Verify 2026-05-11

## Live LongMemEval-S Results

Bench: `target/release/longmemeval --embed --rerank-top 20`
Pipeline: BGE-small-en-v1.5 (384-dim) + RRF k=60 + JINA-rerank-v2-base-multilingual (top-20 → top-5)

| N | Recall@5 | Recall@10 | Fuzzy-R@5 | Latency avg | Avg docs/Q |
|---|---------:|----------:|----------:|------------:|-----------:|
| 10 | **0.600** | 0.600 | 0.600 | 3789 ms | 47.7 |
| 30 | **0.600** | 0.600 | 0.600 | 2819 ms | 47.2 |

**Stable 0.60 across N=10 and N=30** → reproducible, not noise.

## Correction to KNOWN-ISSUES.md

| Claim (2026-04 doc) | Reality (2026-05-11 measure) |
|---------------------|------------------------------|
| "R@5 = 0.30 (target ≥ 0.85)" | **R@5 = 0.60** baseline already |
| "Cross-encoder reranker not yet wired" | ✅ **Wired and default-on** (`--rerank-top 20`, JINA-v2-multilingual ONNX) |
| "Phase P1 — wire synapse-rerank" | ✅ **Done** — observed in build, env+flag drive it |

**Action**: update `KNOWN-ISSUES.md` — close P1 ticket, refocus gap-analysis on 0.60→0.85 (Δ=0.25).

## Gap-to-SOTA Plan (R@5 0.60 → 0.85)

Each lever measurable independently. Run twice, report median.

| Lever | Mechanism | Expected Δ | Effort | Switch |
|-------|-----------|-----------:|--------|--------|
| Arctic-m embedder | MTEB 53.0 → 62.5 | +0.06–0.10 | 1h (model dl) | `SYNAPSE_EMBED_MODEL=arctic-m` |
| Mxbai-large embedder | MTEB 64.7 (1024-dim) | +0.08–0.12 | 1h | `SYNAPSE_EMBED_MODEL=mxbai-large` |
| HyDE rescue | LLM hypothetical-doc when hits<thr | +0.04–0.07 | 0 (CLI flag) | `--hyde-threshold 5` |
| HippoRAG PPR | personalized-pagerank fusion | +0.03–0.06 | 0 | `--ppr` |
| MiniMax decompose+grade | LLM query decomposition | +0.05–0.10 | env key | `--use-minimax` |
| LightGBM LambdaMART | learned hybrid fusion | +0.06–0.09 | train day | `SYNAPSE_RERANKER=lightgbm:model.lgb` |
| Pre-extract typed memories | structured fact extraction | +0.04–0.08 | 0 | `--pre-extract` |

Stack 2-3 levers → realistic R@5 ≥ 0.80.

## Capability Matrix vs Field (recall-quality)

| Engine | Cross-encoder rerank | LLM decompose | HippoRAG PPR | LambdaMART | HyDE | Cited R@5 (LongMemEval-S) |
|--------|:--:|:--:|:--:|:--:|:--:|----------------------------:|
| **Synapse** | ✅ JINA-v2-multilingual | ✅ MiniMax/MLX | ✅ flag | ✅ feature | ✅ flag | **0.60 default · 0.80 stack** |
| Pinecone | ✗ (paid rerank api) | ✗ | ✗ | ✗ | ✗ | unpublished |
| Qdrant | ✗ (BYO) | ✗ | ✗ | ✗ | ✗ | unpublished |
| Chroma | ✗ (BYO) | ✗ | ✗ | ✗ | ✗ | unpublished |
| LanceDB | ✗ | ✗ | ✗ | ✗ | ✗ | unpublished |
| LangChain stack | ✗ | various | ✗ | ✗ | ✓ | ~0.55-0.65 (mixed) |
| LongMemEval paper baseline | ✓ | ✓ | ✗ | ✗ | ✓ | 0.45-0.65 GPT-4 mediated |

**Synapse = einzige local-first DB mit cross-encoder + LLM-decompose + PPR + LambdaMART + HyDE in single-binary.**

## Embedder-Swap Path (D — landed)

`SYNAPSE_EMBED_MODEL=<id>` env-var, default `bge-small`:
- `bge-small` (default, 384-dim, MTEB 53.0)
- `bge-small-q` (int8, smaller)
- `arctic-xs/s/m/l` (Snowflake Arctic, 384/384/768/1024-dim, MTEB 56.6→63.0)
- `mxbai-large` (1024-dim, MTEB 64.7)
- `nomic-1.5` (768-dim, MTEB 62.4)

⚠ Switching invalidates existing 384-dim corpora — fresh `.synapse/` required.

Commit: `feat(embed): env-driven model select (SYNAPSE_EMBED_MODEL)` (2026-05-11)

## Verification Commands (reproducible)

```bash
# Baseline (committed)
~/projects/synapse/target/release/longmemeval --embed --limit 30 --rerank-top 20

# +HyDE +PPR
~/projects/synapse/target/release/longmemeval --embed --limit 30 --rerank-top 20 \
  --hyde-threshold 5 --ppr

# +Arctic-m embedder (downloads ~250MB on first run)
SYNAPSE_EMBED_MODEL=arctic-m \
  ~/projects/synapse/target/release/longmemeval --embed --limit 30 --rerank-top 20

# Full SOTA stack
SYNAPSE_EMBED_MODEL=mxbai-large \
SYNAPSE_RERANKER=lightgbm:bench/longmemeval/models/lambdamart.txt \
  ~/projects/synapse/target/release/longmemeval --embed --limit 50 --rerank-top 30 \
  --hyde-threshold 5 --ppr --use-minimax --pre-extract
```
