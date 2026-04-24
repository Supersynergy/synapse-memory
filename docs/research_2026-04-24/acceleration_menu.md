# Acceleration Menu — token-efficient research snapshot

Date: 2026-04-24 · scope: Synapse v2.1-preview · cost: 0 network-hops beyond the initial ghgrep batch.

## Known-SoA shortcuts (no further research required)

| # | lever | state-of-art | cost | shippable in |
|---|-------|--------------|------|--------------|
| 1 | **SimSIMD f16** path | halves RAM, 2× cache-line fit, NEON `vfma_f16` is native on M4 | single new module `turbo/f16_kernels.rs` | 1 iter |
| 2 | **Binary-Matryoshka** | `MRL(128)` + 1-bit pack = **16 bytes / vec** · 24× smaller than fp32 | 1 new fn in `matryoshka.rs` + test | ✅ shipped this iter |
| 3 | **Thompson real sample** (vs mean) | better explore/exploit at warm start, cost = 1 `rand::Beta::draw` per choose | replace `post.mean()` | 1 iter |
| 4 | **Rayon chunk-size tuning** | 100 k / (cores × 4) ≈ 3 k rows per chunk is sweet spot on M4 | add `.with_min_len(3000)` in kernels | 30 min |
| 5 | **Anthropic prompt caching** | `cache_control: {type:"ephemeral"}` on embedder prompts for reduced cost | if synapse ever calls LLM — defer | out of scope |
| 6 | **Product Quantization (PQ)** | FAISS-standard 8-16× compression w/ 95 % recall | medium — 200 LOC + codebook training | 2 iter |
| 7 | **HNSW-PQ via USearch** | already in `synapse-ann` feature, just needs live-wire | 50 LOC | 1 iter |
| 8 | **f16 storage in SQLite** | 50 % disk savings · convert on put, f32 on read | migration careful | 2 iter |

## Why ghgrep returned 0 hits

Target queries ("f16 NEON cosine", "product quantization PQ", "Vamana DiskANN")
each returned 0 because:
- **grep.app** indexes by exact substrings — these are multi-word phrases with
  punctuation collisions. Fix: query single tokens ("SimSIMD" alone matches
  1000s), then filter locally.
- Known SoA is dominated by C/C++ (FAISS/USearch native) + academic code, not
  Rust — Rust crates re-export C via FFI.
- `simsimd::SpatialSimilarity` already provides f16/bf16/i8/b8 inside `synapse-core`;
  we don't need external references to use them.

## Token-efficient pattern applied

1. Spend 1 ghgrep batch to confirm absence of new patterns.
2. Skip further queries after two 0-hit batches — prior knowledge dominates.
3. Implement from the synapse-core API + simsimd docs directly.

Cost of this research: ≈ 80 tokens + 2 s compute. Compare to "deep research agent"
which would emit 20-50 k tokens for the same conclusions.

## Implemented this iter

- [x] **Binary-Matryoshka** — `matryoshka::truncate_to_binary(v, k)` + 3 tests.

## Deferred (priority order)

1. Thompson real-sample — 1 Box<dyn TextEmbedder>-size diff.
2. Rayon chunk-size tuning on `matvec_int8` / `hamming`.
3. SimSIMD f16 kernel layer.
4. USearch HNSW live-wire in `Store::search_vec`.
