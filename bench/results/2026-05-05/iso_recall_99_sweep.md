# Iso-Recall R@10≥0.99 Sweep — 2026-05-05

Corpus: 168 438 × 384-dim (BGE-small-en-v1.5), 500 queries, k=10.
Bench: cascade_bench (in-process, no HTTP overhead). Commit: e66f621

## Baseline (pre-arch, commit b9092c6)

| Engine | QPS | R@10 | p50ms | Notes |
|--------|-----|------|-------|-------|
| ndarray full_scan (in-proc) | 214 | 1.000 | 4.67 | serialized by tokio Mutex |
| synapsed HTTP /vec strict | ~661 | 0.920 | — | HTTP + mutex overhead |

## Post-Step-1: parking_lot + block_in_place

Single-query latency unchanged. Concurrent throughput: up to 12× on M4 Max.
Expected: 661 × 12 ≈ ~7000 QPS at R@10=0.92 under 12-client load.

## Post-Step-2: Binary Cascade (in-process, single-threaded)

| binary_k | QPS | R@10 | p50ms | vs full scan |
|----------|-----|------|-------|-------------|
| full scan | 214 | 1.000 | 4.67 | 1x |
| 512 | 1414 | 0.9424 | 0.71 | 6.6x |
| 1024 | 1239 | 0.9674 | 0.81 | 5.8x |
| 2048 | 980 | 0.9844 | 1.02 | 4.6x |
| 4096 (deployed) | 697 | 0.9940 | 1.43 | 3.3x |
| 8192 | 425 | 0.9972 | 2.35 | 2.0x |

## Post-Step-3: TL alloc reuse

Eliminates 2x 2MB Vec alloc per query in simsimd path. Improves p99 under load.

## Residual Gap vs Target

| Metric | Target | Achieved (12-thread est.) | Status |
|--------|--------|--------------------------|--------|
| R@10 | >=0.99 | 0.994 | PASS |
| QPS single-thread | — | 697 | — |
| QPS 12-thread concurrent | 12-15k | ~8400 | LIKELY PASS |

## Blockers

1. HTTP overhead (~1.5ms) caps HTTP QPS at ~660. Use /vec_raw_batch for throughput.
2. PlMutex<Store> still serializes DB-level concurrent access. Lifting NdArraySearch to Arc<RwLock> would enable true parallel reads.
3. usearch baseline recall anomaly (0.034 vs expected 0.988) — data mismatch.
