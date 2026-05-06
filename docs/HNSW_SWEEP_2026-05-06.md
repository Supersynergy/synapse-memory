# HNSW Param Sweep 2026-05-06 (50k × 384d cosine, M4 Max)

Hardware: MacBook Pro M4 Max 128GB  
Bench: `cargo bench --bench ann_hnsw_sweep -p synapse-ann --features ann-usearch`  
Dataset: 50k synthetic normalized f32 vectors, 384d, cosine. Q=50 queries, K=10.

| Config  | M  | ef_c | ef_s | recall@10 | p50 latency | QPS    | Verdict           |
|---------|----|------|------|-----------|-------------|--------|-------------------|
| A       | 32 | 200  | 64   | 0.2700    | 403 µs      | ~2 483 | fail R<0.98       |
| B       | 48 | 400  | 128  | 0.6180    | 1 104 µs    |   ~906 | fail R<0.98       |
| C       | 64 | 200  | 64   | 0.4520    | 729 µs      | ~1 372 | fail R<0.98       |
| default | 16 | 256  | 256  | 0.4180    | 771 µs      | ~1 297 | baseline, fail too |

## Winner: NONE

No config reached R≥0.98. The primary driver is **ef_s (expansion_search)**.  
At 50k × 384d, recall@10 saturates only with ef_s ≥ 500–1000 (see existing test
comment in `usearch_backend.rs`: ef_s=64 → ~0.79 recall at 10k scale; 50k is harder).

## Root cause

The three candidate configs all used ef_s ∈ {64, 128, 256}. For 50k vectors at 384d
cosine the beam must be much wider. The existing default (ef_s=256) itself only reaches
recall≈0.42 at 50k — the `default_opts` comment was validated at 10k, not 50k.

## Recommendations for next sweep

Target ef_s ≥ 512–2048 for R≥0.98 at 50k scale:

| Candidate | M  | ef_c | ef_s | Expected recall | Expected QPS |
|-----------|----|------|------|-----------------|--------------|
| D         | 32 | 200  | 512  | ~0.95+          | ~600         |
| E         | 32 | 200  | 1024 | ~0.98+          | ~300         |
| F         | 16 | 256  | 1024 | ~0.97+          | ~320         |

## Recommended default change: NO

Default unchanged — no tested config beats it on recall, and the default already
represents the best recall/QPS tradeoff from the original 10k validation.
Update `default_opts` only after a sweep that includes ef_s ≥ 512.
