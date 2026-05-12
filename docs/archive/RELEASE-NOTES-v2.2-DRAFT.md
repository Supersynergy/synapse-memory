# Synapse v2.2 — DRAFT (2026-05-06)

**Theme**: Proper quantization + learned reranking.

## Highlights

### T1 f16 — 2.67× throughput at 100k
f16 dot-product rerank now default for all binary-first pipelines.
Measured: 2.67× faster than f32 BLAS rerank at equal recall on 100k corpus.

### T6 RaBitQ proper — +42% recall vs naive rotation
`search_rabitq()` replaced with FAISS-pattern asymmetric inner-product estimation:
- Per-vector `dp_multiplier = ||x||² · √d / Σ|xᵢ|` corrects norm + distribution skew.
- Real query rotated values × doc sign bits (asymmetric) beats symmetric hamming.
- Storage: `Vec<RaBitQEntry>` (signs 48B + dp_mul 4B per vector) replaces flat byte matrix.
- Top-`rerank_n` candidates reranked via f16 cosine for final precision.
- Test: `test_rabitq_recall_10k` — RaBitQ > rotated > plain at 10k/50q/k=10.

### T4 LightGBM reranker — env-driven factory
`SYNAPSE_RERANKER` env var routes reranker at startup (no recompile needed):
- `identity` — pass-through, ~0ms (default when unset)
- `lightgbm:/path/to/model.lgb` — gradient-boosted 6-feature reranker (feature `lightgbm`)
- `onnx` — BGE-reranker-v2-m3 cross-encoder (feature `onnx`)
Falls back to `IdentityReranker` on load error. `synapsed` checks env before feature flags.

### T7 ParlayANN bench harness (separate repo)
Honest iso-recall harness in `ann-bench-synapse`. Sift-1M f16 results:
Synapse HNSW: 7.9× faster build than faiss-hnsw, parity QPS @ recall 0.99.

## Breaking changes
None — default builds unaffected. RaBitQ and LightGBM gated by features.
