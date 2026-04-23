# M4 Max Extreme Integration Plan

> "Was noch geht" — gemessen gegen Synapse v2 (Turbo) baseline. Hardware: M4 Max, 12 P-cores + 4 E-cores, 128 GB unified, 546 GB/s bandwidth.

## Hardware audit — was Synapse nutzt vs was geht

| Resource | Synapse heute | Ungenutzt / slack |
|---|---|---|
| 12 P-cores | ✅ rayon | — |
| 4 E-cores | ⚠️ rayon nimmt mit | besser: offload I/O-bound (cache write) → E-cores |
| 40-core GPU | ❌ fastembed ist ONNX CPU | **komplett frei** — Metal/MLX embedder |
| 16-core Neural Engine (ANE) | ❌ 0% | cross-encoder rerank via CoreML |
| AMX matrix coprocessor | ❌ indirect | erreichbar via Accelerate.framework cblas_sgemm |
| 128 GB unified mem | ✅ mmap-friendly | 90 %+ frei — whole-corpus RAM-index möglich bis ~20 M docs |
| 546 GB/s bandwidth | ⚠️ wenig genutzt | Turbo-int8 bandbreiten-bound ≈ 80 GB/s cap |
| SimSIMD NEON i8/b8 | ❌ 0% | ~5–10× dot products vs Rust-scalar |
| ARMv9 SVE2 / SME | ❌ nicht verfügbar | M-series hat bisher nur NEON-4x128 — kein Miss |

## Ranked opportunities — Top 12

| # | Integration | Dev cost | Win | Evidence |
|---|---|---|---|---|
| 1 | **simsimd crate** (dot/cos/hamming/i8/b8) | S (2 h) | 3–10× auf ANN-hot-path | 200× claim, ClickHouse/Dragonfly/USearch nutzen |
| 2 | **metal-candle** embedder (BGE-small Metal) | M (1 d) | embed 8 ms → ~1–2 ms, batch 32 near-const 4 ms | 25.9× vs MLX, benchmarks.md |
| 3 | **Matryoshka (MRL) truncation** 384→128 | S (4 h) | 3× matvec throughput, recall@10 fast unverändert | arXiv 2205.13147, HF blog |
| 4 | **CoreML ANE rerank** (cross-encoder ms-marco-MiniLM-L-6-v2) | M (1 d) | rerank top-20 in 5 ms on ANE (sonst 80 ms CPU) | model schon local |
| 5 | **Accelerate BLAS** für large-batch matmul | S (4 h) | pgemm 2× vs Rust-scalar; AMX auto-routing | `accelerate-src` crate |
| 6 | **USearch v2.15** als ANN backend hinter `ann-usearch` feature | S (3 h) | HNSW schneller als sqlite-vec flat >50 k | simsimd internal |
| 7 | **Binary Matryoshka** (384 MRL → 128 → 1-bit) | S (3 h) | 0.5 M QPS rerank floor | USearch Quint4 precedent |
| 8 | **mmap zero-copy** `codes`/`bits` direkt aus SQLite-BLOB-storage | M (1 d) | spart RAM-doppel + kaltstart 10× | libsql mmap pragma |
| 9 | **Speculative cache** — promote blake3 hits in SIMD-Bloom first | S (3 h) | 50 ns pre-check vs 500 ns redb rtx | wyhash + aht |
| 10 | **ANE chunk-level dense index** via CoreML MiniLM-L6 | L (3 d) | latent-space freshness rerank | experimental |
| 11 | **Metal 4 compute pipeline** (objc2-metal) für topk + argpartition | M (2 d) | topk 10 k → 50 µs | spec only |
| 12 | **Async kqueue put-pipe** für bulk insert | M (1 d) | 50 k docs/s end-to-end | tokio + io-uring-style |

**Realistic next-PR bundle** → #1 + #3 + #5 = 1 Tag Arbeit, 5–10× schneller.

## Decision: implementieren

1. ✅ `synapse-core` + `simsimd` feature flag → `turbo/simsimd_kernels.rs`
2. ✅ `synapse-core/src/matryoshka.rs` — truncate + renormalize
3. ⏳ `metal-candle-embedder` als `turbo/candle_metal.rs` (draft separat)
4. ⏳ ANE rerank via `coremltools → coreml-rs` (draft, braucht Swift bridge)

Mess-Target nach PR: embed 1 doc < 2 ms, int8 matvec 100 k < 300 µs, hamming 1-bit < 80 µs, end-to-end put < 15 ms, search p99 < 1 ms.
