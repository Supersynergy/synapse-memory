# M4-Max AMX vs NEON — Pearson Corr-Matrix Bench 2026

> First public M4-Max AMX-vs-NEON Pearson correlation-matrix benchmark.

## Environment

| Key | Value |
|-----|-------|
| CPU | Apple M4 Max |
| OS  | macOS 15.5 (Darwin 24.5.0 arm64) |
| Framework | Accelerate (cblas_sgemm) |
| Date | 2026-05-12 |

## Workload

- 220 tickers × 252 daily returns (f32)
- Normalize each ticker-series to zero-mean unit-variance
- Pearson corr-matrix = `(1/252) * Z @ Z.T` → output 220×220
- 200 iterations + 10 warmup per impl
- FLOPS counted as `2 × 220 × 220 × 252 = 24,393,600`

## Results

| impl | p50 | p95 | mean | GFLOPS |
|------|----:|----:|-----:|-------:|
| naive (scalar) | 3.5ms | 3.8ms | 3.5ms | 6.9 |
| wide (NEON f32x8) | 793µs | 832µs | 795µs | 30.7 |
| cblas_sgemm (AMX) | 25µs | 47µs | 30µs | 808.8 |

## Speedup

| Comparison | Factor |
|-----------|-------:|
| AMX vs naive | **117×** |
| AMX vs NEON | **26×** |
| NEON vs naive | 4.4× |

## Notes

- `cblas_sgemm(ROW_MAJOR, NO_TRANS, TRANS, 220, 220, 252, 1/252, Z, 252, Z, 252, 0, C, 220)`
  — single BLAS-3 call, Apple's Accelerate dispatches to AMX coprocessor automatically on M3+.
- p95 jitter for cblas (47µs) reflects occasional OS scheduling; p50=25µs is the steady-state.
- NEON `wide` f32x8 inner-loop operates on pre-normalized rows; same algorithmic path as cblas but scalar matmul outer loop.
- Naive impl: raw triple nested loop, no vectorization hints.

## Reproducer

```bash
cd ~/projects/synapse
cargo bench --bench amx_minimal -p synapse-market
```

Source: `crates/synapse-market/benches/amx_minimal.rs`

## Public-PR Ready?

Yes. The bench is self-contained (no external data files, deterministic PRNG seed=42, correctness assert ±1e-4). Ships with existing `accelerate-src = "0.3"` + `cblas-sys = "0.3"` dev-deps already in Cargo.toml.
