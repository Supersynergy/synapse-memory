# PHASE-3: SimSIMD Coverage Map

Date: 2026-04-25 · Owner: Team Epsilon ε1/ε2/ε3
Source: subagent a8b48888065ece43b

## Goal
Apply NEON/AVX SimSIMD kernels to every scalar hot-path. Current usage = bench-only. Production paths still scalar.

## Bench Target
hybrid p50: 2ms → **<0.5ms** at 137k docs (4× compound).

## Top-3 to Ship Week 5 (Highest ROI)

### 1. RRF Score Merging (db.rs:556-586)
- **Lines 562-575:** scalar div+add over 2k results
- **simsimd kernel:** f64 add batch + parallel chunks
- **Expected:** **8-12× speedup**
- **Recall risk:** ZERO (RRF = score fusion, post-rank)

### 2. Vector Distance → Score Conversion (db.rs:510-554)
- **Line 549:** scalar `1.0 / (1.0 + dist)` over 100 hits
- **simsimd kernel:** reciprocal vec batch
- **Expected:** **5-7× speedup**
- **Recall risk:** ZERO (transform monotone)

### 3. sqlite-vec Embedding Load (db.rs:391-408)
- **Lines 402-404:** scalar byte→f32 unpacking on 137k embeddings
- **simsimd kernel:** byte-shuffle + f32 cast batch
- **Expected:** **4-6× speedup** (cold rebuild path)
- **Recall risk:** ZERO (data prep, not score)

## Recall-Safe vs Recall-Risk

**MUST NOT QUANTIZE (recall guards required):**
- HNSW link traversal (ann.rs:96-108) — cosine for neighbors
- BM25 scoring (db.rs:445-461) — SQL-side compute

**Safe to quantize (no recall impact):**
- RRF reciprocal fold
- Distance→score post-conversion
- Embedding byte unpacking
- COUNT/SUM aggregations

## Build Matrix

```rust
// crates/synapse-core/Cargo.toml
[features]
simsimd = ["turbo", "dep:simsimd"]   # default-ON for ARM macOS

// crates/synapse-core/src/lib.rs
#[cfg(feature = "simsimd")]
pub use turbo::simsimd_kernels;

#[cfg(not(feature = "simsimd"))]
pub use turbo::scalar_kernels as simsimd_kernels;
```

| Platform | simsimd | Backend |
|---|---|---|
| macOS ARM64 (M-series) | auto-ON | NEON kernels |
| Linux x86_64 + AVX2 | auto-ON | AVX2 kernels |
| Linux x86_64 no AVX2 | OFF | scalar + rayon |
| Other | OFF | scalar |

## Top-7 Candidates (full table)

| # | Path | File:Line | Scalar Δ | SIMD Gain | Ship Week |
|---|---|---|---:|---:|:-:|
| 1 | RRF merge | db.rs:562-575 | 1.2ms | 8-12× | 5 ⭐ |
| 2 | Distance→score | db.rs:549 | 0.3ms | 5-7× | 5 ⭐ |
| 3 | byte→f32 unpack | db.rs:402-404 | 100ms cold | 4-6× | 5 ⭐ |
| 4 | BM25 normalize | db.rs:455 | 0.4ms | 3-4× | 6 |
| 5 | HNSW probe (i8) | ann.rs link traversal | 8ms | 2-3× | 6 |
| 6 | COUNT/SUM aggs | search_*.rs | varies | 4-8× | 7 |
| 7 | BLAKE3 batch dedup | db.rs put_batch | 50ms ingest | 4× | 7 |

## Validation Bench (DoD)
- Hybrid p50 ≤ 0.5ms @ 137k docs
- Recall@10 holds at 1.000 (dense) and ≥0.95 (i8 quant)
- No path regresses >5%
- ARM Mac + Linux x86_64 + Linux ARM all pass

## Status: Ready for Week 5 implementer sprint (3 PRs).
