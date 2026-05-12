# SIFT-1M Benchmark — Synapse Native Indices (2026-05-12)

**Dataset**: ann-benchmarks `sift-128-euclidean.hdf5`
- 1 000 000 train × 128d, 10 000 test queries (1k used here)
- Metric: L2-normalised → cosine (ground-truth recomputed brute-force)
- URL: http://ann-benchmarks.com/sift-128-euclidean.hdf5 (IRISA, public)

**Hardware**: Apple M4 Max, 128GB RAM · Rust release LTO · SimSIMD NEON
**Source**: `ann-bench-synapse/src/sift1m_synapse.rs`

---

## Results — Synapse Brute-Force (linear SIMD scan)

| Mode | Build s | RAM MB | R@10 | R@100 | p50 ms | p99 ms | QPS |
|------|--------:|-------:|-----:|------:|-------:|-------:|----:|
| synapse-f32    |  0.0 | 512 | 0.9999 | 1.0000 | 18.8 | 24.5 |  53 |
| synapse-f16    |  0.2 | 256 | 0.9993 | 0.9992 | 22.9 | 34.6 |  43 |
| synapse-i8     |  0.1 | 128 | 0.9687 | 0.9792 |  5.4 |  6.4 | 186 |
| synapse-rabitq |  4.1 |  48 | 0.866  | 0.690  |  4.3 |  5.0 | 231 |

---

## Comparison vs Published Numbers

Synapse = **brute-force** (linear scan). Fair peer = faiss-flat, not HNSW.

| Engine | Type | Corpus | QPS | R@10 | Source |
|--------|------|--------|----:|-----:|--------|
| faiss-flat f32 (1-thread) | brute | SIFT-1M | ~337 | 1.000 | FINAL_RESULTS.md |
| hnswlib M=16 ef=128 | HNSW | SIFT-1M | ~5 000 | 0.990 | ann-benchmarks.com |
| usearch-f16 HNSW | HNSW | 50k×384d | ~16 667 | 0.982 | bench_10way_2026-05-06 |
| **Synapse-f32** | brute | SIFT-1M | 53 | 0.9999 | this |
| **Synapse-i8** | brute | SIFT-1M | 186 | 0.969 | this |
| **Synapse-rabitq** | cascade | SIFT-1M | 231 | 0.866 | this |

Multi-thread scaling: Synapse uses rayon — f32 53 QPS × 16 cores ≈ 800+ QPS.
faiss-flat OMP=16 ≈ 3 000–5 000 QPS. Synapse still ~4–6× behind faiss-flat multi-thread.

---

## Key Findings

**i8 sweet spot**: 3.5× faster than f32, 4× RAM, R@10=0.969.
**RaBitQ** highest QPS (231), 10× RAM vs f32, but R@100=0.69 — Hamming prefilter too narrow at 128d/1M. Fix: `hamming_n=5000` (default is k×50=500).
**f16 slower than f32 at 128d** — half-float NEON overhead dominates at low dim. f16 wins at ≥256d where bandwidth matters more (validated at 384d: 2× faster).

**vs faiss-flat**: Synapse-i8 (186 QPS) < faiss-flat (337 QPS) single-thread — not faster for pure-ANN.
**vs HNSW**: 20–100× slower — expected. Synapse is hybrid BM25+vec RRF memory, not pure ANN.
Synapse ships optional usearch HNSW (`--features ann-usearch`) for corpus >500k.

---

## Notes

- f16: InMemoryF16Index — NEON half-float SimSIMD, 50% RAM vs f32
- i8: InMemoryI8Index — NEON int8 dot, 75% RAM, R@10 ~3% loss
- rabitq: RaBitQIndex — 1-bit Hamming → RaBitQ rerank → f32 verify; needs wider hamming_n at 128d
- For HNSW comparison see RAW_ANN_BENCH_2026-05-11.md (usearch backend)
