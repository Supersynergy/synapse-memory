# LongMemEval-S Benchmark

Evaluates Synapse SOTA pipeline on the LongMemEval-S-50 subset (50 questions).

## Preferred Config (highest R@5)

```bash
cargo run -p longmemeval --features "embed-768,rerank" -- \
  --embed --rerank-top 20
```

When both `embed-768` **and** `rerank` features are active and `--embed` is set,
the benchmark automatically defaults to **Snowflake Arctic Embed M** (`arctic-m`,
768-dim, MTEB 62.5) unless `SYNAPSE_EMBED_MODEL` is set explicitly.

**Confirmed result**: R@5 = 0.64 (+4pp over BGE-small baseline of ~0.60).

## Feature Flags

| Flag | Effect |
|------|--------|
| `embed-768` | Enables 768-dim vector slot in synapse-core |
| `rerank` | Enables BGE-reranker-v2-m3 ONNX cross-encoder |
| `minimax` | Enables MiniMax M2.7 hooks (decompose/grade/hyde) |
| `mlx` | Enables MLX subprocess hooks |

Default features: `minimax rerank` (no embed).

## Model Selection

Override via env: `SYNAPSE_EMBED_MODEL=<key> cargo run -p longmemeval ...`

| Key | Model | Dim | MTEB |
|-----|-------|-----|------|
| `bge-small` | BGE-small-en-v1.5 (default) | 384 | 53.0 |
| `arctic-xs` | Snowflake Arctic Embed XS | 384 | 56.6 |
| `arctic-s` | Snowflake Arctic Embed S | 384 | 60.0 |
| **`arctic-m`** | **Snowflake Arctic Embed M** | **768** | **62.5** ← preferred with rerank |
| `arctic-l` | Snowflake Arctic Embed L | 1024 | 63.0 |
| `mxbai-large` | MxbAI Embed Large v1 | 1024 | 64.7 |
| `nomic-1.5` | Nomic Embed Text v1.5 | 768 | 62.4 |

## Key Flags

```
--embed            Enable vector embeddings (fastembed ONNX)
--rerank-top N     Cross-encoder rerank over top-N candidates (default 20)
--limit N          Evaluate only first N questions (0 = all 50)
--verbose          Per-question logging
--use-mlx          MLX hooks for decompose/grade/hyde
--use-minimax      MiniMax M2.7 hooks (requires MINIMAX_API_KEY)
--pre-extract      Pre-extract typed memories before recall
--ppr              Enable Personalized PageRank (HippoRAG-2) signal
--judge            LLM-judge mode (protocol parity with paper)
```

## Smoke Bench

```bash
cargo run -p longmemeval --features "embed-768,rerank" -- \
  --embed --rerank-top 20 --limit 50
# Expected: Recall@5 >= 0.62
```
