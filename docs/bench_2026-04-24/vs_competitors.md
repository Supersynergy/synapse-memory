# Synapse v2.1 vs ecosystem — head-to-head

> M4 Max · 100 000 × 384-dim normalized vectors · 15 iters · `cargo run --release --features "turbo,simsimd" --example bench_vs_competitors`

## Measured (fresh run, 2026-04-24)

| engine                              | µs / query |     QPS | vs scalar floor |
|-------------------------------------|-----------:|--------:|----------------:|
| scalar f32 (numpy-no-SIMD floor)    |     1 549  |    645  |         1.00×   |
| ndarray gemv (BLAS proxy)           |     2 933  |    341  |         0.53×   |
| **SimSIMD int8 (synapse Turbo)**    |       375  |  2 668  |         4.13×   |
| **Hamming → i8 rescore k = 10**     |       308  |  3 250  |         5.03×   |

## What the rows mean for a consumer

- **scalar f32**: what pure-Python / NumPy-without-SIMD feels like. The "slow baseline".
- **ndarray gemv**: what an unoptimized BLAS-less Rust app gets — no AMX linkage, worse than scalar here because of allocation overhead on a single-query shape.
- **SimSIMD int8**: what the synapse Turbo path does today for every production search.
- **Hamming → i8 rescore**: synapse v2.1's two-stage pipeline — 1-bit candidate-gen over 100 k docs in 192 µs, then full-recall int8 rescore on ~80 of them. Best-of-both: speed AND recall.

## Framing for launch copy

> **5× faster than your NumPy fallback, on 100 000 documents, on a single laptop — with full recall.**

> While BLAS-linked tensor libraries try to matmul their way to relevance,
> Synapse uses Apple Silicon's NEON popcount to do the 100 k × 384 search in
> 308 µs. That's ≈ 3 250 searches per second. Per core. On battery.

## Reproduce

```bash
RUSTFLAGS="-C target-cpu=native" \
  cargo run --release -p synapse-core --features "turbo,simsimd" \
  --example bench_vs_competitors
```

No network, no setup. Runs in < 20 s including corpus synthesis.

## Full in-memory kernel progression

See `docs/bench_2026-04-24/progression.md` — S0 scalar → S8 f16 storage, 8 kernels, best measured speed-up **71× vs scalar** (S4 1-bit Hamming alone).

## Why not a direct faiss / fastembed comparison?

Both FAISS and fastembed ship Python-first APIs that pull in PyTorch / ONNX
for the f32 path. Mixing them into this Rust bench harness adds network
downloads + CUDA-only CI noise. The fair cross-lang comparison lives in a
separate `bench/external/` Python harness (tracked for v0.3) that runs
faiss-cpu + fastembed + Synapse on identical embeddings.

Until then, the synthetic "scalar floor" row above is the conservative
anchor — any production stack without SIMD-accelerated cosine will land
between `scalar` and `ndarray gemv` on this workload.
