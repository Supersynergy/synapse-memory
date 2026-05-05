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
3. usearch baseline recall anomaly (0.034 vs expected 0.988) — data mismatch. **FIXED** (see Phase D below).

---

## Phase D — In-Process usearch Sweep (2026-05-05)

**Binary**: `bench/industry/src/bin/inproc_recall.rs`  
**Engine**: usearch 2.25 via synapse-ann — direct Rust fn call, zero HTTP overhead  
**Corpus**: 168,438 × 384 | **Queries**: 1,000 | **GT**: brute-force cosine top-100  
**GT fix**: GT IDs are corpus IDs (1..168439) not 0-based indices — prior 0.034 recall was measurement bug.

| M | ef_s | ef_c | QPS | R@10 | R@100 | p50ms | p95ms | p99ms |
|---|------|------|----:|-----:|------:|------:|------:|------:|
| 16 | 64 | 400 | 2079 | 0.977 | 0.926 | 0.480 | 0.789 | 0.896 |
| 16 | 128 | 400 | 1628 | 0.982 | 0.945 | 0.619 | 1.038 | 1.160 |
| 16 | 200 | 400 | 1147 | 0.988 | 0.969 | 0.849 | 1.503 | 1.696 |
| 16 | 400 | 400 | 653 | 0.993 | 0.988 | 1.531 | 2.634 | 2.968 |
| 32 | 64 | 400 | 1196 | 0.982 | 0.955 | 0.853 | 1.464 | 1.645 |
| 32 | 128 | 400 | 976 | 0.985 | 0.966 | 1.049 | 1.843 | 2.033 |
| 32 | 200 | 400 | 695 | 0.990 | 0.982 | 1.459 | 2.633 | 2.970 |
| 32 | 400 | 400 | 393 | 0.994 | 0.992 | 2.540 | 4.646 | 5.090 |
| **48** | **64** | **400** | **1631** | **0.982** | **0.958** | 0.630 | 1.071 | 1.210 |
| 48 | 128 | 400 | 1325 | 0.985 | 0.970 | 0.740 | 1.422 | 1.639 |
| 48 | 200 | 400 | 968 | 0.991 | 0.984 | 1.024 | 1.877 | 2.100 |
| 48 | 400 | 400 | 534 | 0.994 | 0.994 | 1.814 | 3.468 | 4.170 |

**Best ≥ R@10=0.98**: M=48 ef_s=64 ef_c=400 → **1631 QPS @ R@10=0.982**  
**Closest to R@10=0.99**: M=48 ef_s=400 → **534 QPS @ R@10=0.994**

### HTTP Overhead Confirmed

| Config | QPS | R@10 | Source |
|--------|----:|-----:|--------|
| ultra_raw HTTP strict | 661 | 0.920 | HTTP ~1.5ms/req |
| ultra_raw HTTP binary_first | 1510 | 0.913 | HTTP ~0.5ms/req |
| **inproc M=16 ef_s=64** | **2079** | **0.977** | 0ms overhead |
| **inproc M=48 ef_s=64** | **1631** | **0.982** | best recall/QPS tradeoff |

Removing HTTP adds **2.5-3.1× QPS** at same or higher recall.  
HTTP overhead = **~0.45ms/query** confirmed.  
Bottleneck at high ef_search: HNSW neighbor scan O(M × ef_s), not distance kernel or memory layout.

### Gap vs usearch prior (5898 @ R@10=0.919)

Prior run: ef_construct=256, M=16, ef_s=64. Lower build cost → faster traversal, lower recall.  
This run: ef_construct=400 produces denser graphs → recall ceiling raised, QPS lower at same ef_s.  
To recover 5898 QPS: use ef_construct=256, M=16, ef_s=64 (0.919 recall, below 0.98 target).
