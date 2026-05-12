# Synapse v2.1.0 — 2026-05-05

**Theme**: Recall-king on Mac. Hybrid auto-route. Honest benchmarks.

## Highlights

- **Hybrid auto-route** by `target_recall` — cascade / usearch HNSW / binary_first
- **Cascade R@10=0.994 @ 697 QPS** in-proc (binary_k=4096, NEON SimSIMD)
- **usearch HNSW M=48 ef_s=64**: 1631 QPS @ R@10=0.982
- **Binary_first 12-worker**: 5958 QPS @ R=0.888 (HTTP `/vec_raw_batch`)
- **SuperML param predictor** (TabPFN/CatBoost) for HNSW auto-tune
- **Mac-native perf**: Accelerate GEMM, SimSIMD f16 NEON, single-thread SIMD scan, embedder mutex-drop, RwLock hot-path lift

## Routing

```rust
auto_route(target_recall) -> SearchBackend
  ≥0.98 → Cascade        // 697 QPS, R=0.994
  0.94..=0.97 → UsearchHnsw  // 1631 QPS, R=0.982
  <0.94 → BinaryFirst     // 5958 QPS, R=0.888
```

HTTP: `/vec_raw?target_recall=0.99`. RecallParams.target_recall optional (default 0.98).

## Benchmarks (168k × 384 BGE-small, M4 Max, 1000 queries)

| Engine | QPS | R@10 | Build |
|---|---:|---:|---:|
| Synapse cascade | 697 | 0.994 | instant |
| Synapse binary_first 12c | 5958 | 0.888 | instant |
| usearch M=48 ef_s=64 | 1631 | 0.982 | 86s |
| usearch M=16 ef_s=400 | 653 | 0.993 | 86s |
| Lance IVF_HNSW_SQ | — | 0.86 | 15s |
| Qdrant M=16 ef=128 (20k cap) | 86 | 1.000 | 9.6s |

Phase D-H benches: `bench/results/2026-05-05/`

## Industry positioning

- **NOT raw HNSW QPS king** — usearch/hnswlib win at R<0.95
- **Recall-king at R≥0.99 in-proc** — cascade wins both QPS and build time
- **Mac-native differentiator** — Accelerate, NEON f16, MLX-ready
- **Hybrid + typed memory** — agent-memory store, not raw vector DB

## Negative results (honest)

- **RaBitQ random rotation** in cascade: -27% R@10 on normalized BGE embeddings. Already maximally entropic — rotation destroys structure. Reverted. Doc: `bench/results/2026-05-05/rabitq_rotation_attempt.md`
- **libSQL 0.9.30 migration**: PAUSE — SQLITE_MISUSE on init. Stay rusqlite.
- **HTTP 10k QPS @ R≥0.98**: not reached. Bandwidth-bound. Use in-proc cascade or HNSW.

## Memory bandwidth ceiling (M4 Max)

168k × 384 × 4 = 258 MB scan. ~200 GB/s practical → 1.3ms/query min → 770 QPS/core × 12 = 8400 QPS aggregate hard ceiling for brute-force f32 R=1.0.

## v1.2 backlog

- VectorChord/RaBitQ4 IVF spike (>1M corpus regime)
- DiskANN-rs evaluate post-1.0 (Microsoft Rust port)
- ParlayANN parallel build (-90% build time)
- MLX Metal embedder (production wire, currently behind feature)

## Commits since v2.0.0

```
ef8b139 feat(sota): hybrid auto-route by target_recall
9400892 feat(ffi): cdylib direct call
1a1e714 feat(uds): bincode binary frame vec socket
54362ff perf(batch): Accelerate BLAS GEMM — 24-48× QPS
5e67a8c perf(f16): simsimd NEON f16 cosine
e4e0ef2 perf(ndarray): simsimd cosine in hamming rerank
b519652 perf(embed): drop mutex guard before ONNX
76d408b perf(ndarray): single-thread SIMD scan
f594b5c feat(ultra): RwLock-lifted hot path
d479dd2 feat(ultra): HNSW backend (usearch 2.25)
e66f621 perf: binary cascade + TL alloc reuse
723dd0a fix(license): jsonwebtoken rust_crypto + mutex poison recovery
ec6e6b6 feat(ann): SuperML param predictor
```

## Constraint adherence

- Pure Rust ✓ (MLX subprocess, off critical path)
- Additive only ✓ (no breaking schema, RecallParams new field optional)
- Ed25519/CRDT/MCP untouched ✓
- 11/11 SOTA tests + 97/97 synapse-core lib tests pass
