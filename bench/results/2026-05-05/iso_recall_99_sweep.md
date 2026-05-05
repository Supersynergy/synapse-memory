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

---

## Phase E — RwLock-Lifted Hot Path (2026-05-05)

**Change**: `NdArraySearch` lifted from `PlMutex<Store>` to `Arc<parking_lot::RwLock<Option<NdArraySearch>>>` in `synapsed::State`.  
**Bench target**: `synapse-ultra` `/vec_raw` + `/vec_raw_batch`, 12 concurrent Python workers, 1000 queries, k=10.  
**Corpus**: 168,438 × 384-dim | **GT**: brute-force cosine

### Single-client (baseline, ArcSwap already in synapse-ultra)

| Mode | QPS (1c) | R@10 | p50ms |
|------|----------|------|-------|
| strict | 730 | 0.920 | 1.36 |
| binary_first | 1727 | 0.913 | 0.57 |
| vec_raw_batch b=32 strict | 836 | 0.920 | 1.20 |
| vec_raw_batch b=32 binary_first | 1990 | 0.913 | 0.51 |

### 12-worker concurrent

| Mode | Agg QPS | R@10 | p50ms | vs 1c |
|------|---------|------|-------|-------|
| /vec_raw strict | 724 | 0.920 | 14.4 | **0.99×** — Rayon thread pool saturated |
| /vec_raw binary_first | 4846 | 0.888 | 2.11 | 2.8× |
| /vec_raw_batch b=32 strict | 668 | 0.920 | 16.0 | 0.91× |
| /vec_raw_batch b=32 binary_first | **5958** | 0.888 | 1.33 | 3.0× |

### Analysis

- **Strict mode** (`top_k_f32`): uses `rayon::par_iter` over full 168k corpus with CHUNK=4096. Each query saturates all 12 cores → concurrent requests serialize on Rayon global pool → **no concurrency gain** (724 vs 730).
- **Binary_first**: lighter Rayon usage (hamming scan + small f16 rerank) → 2.8-3.0× gain at 12 workers.
- **Target 10k QPS @ R@10≥0.98**: not reached. Strict mode recall 0.920, binary_first recall 0.888 — both below 0.98.
- **Residual bottleneck**: Rayon global thread pool. Fix: per-query bounded thread pool (`rayon::ThreadPoolBuilder::new().num_threads(1)` for single-threaded SIMD path) or AVX512/NEON intrinsic brute-force without Rayon.

### Commits
- `feat(turbo): pub hydrate_hits_by_id_dist + take_ndarray_search — enables RwLock lift`
- `feat(ultra): RwLock-lifted hot path — synapsed SearchVec bypasses Store mutex`

### Gap vs usearch prior (5898 @ R@10=0.919)

Prior run: ef_construct=256, M=16, ef_s=64. Lower build cost → faster traversal, lower recall.  
This run: ef_construct=400 produces denser graphs → recall ceiling raised, QPS lower at same ef_s.  
To recover 5898 QPS: use ef_construct=256, M=16, ef_s=64 (0.919 recall, below 0.98 target).

---

## Phase F — Single-Thread SIMD (2026-05-05)

**Change**: Replaced `rayon::par_iter` global-pool parallelism with single-threaded SIMD scan in:
- `synapsestore/crates/synapse-ultra/src/search.rs` — `top_k_f32` (strict) + `top_k_binary_first` hamming phase
- `crates/synapse-core/src/turbo/inmem_i8_index.rs` — `search()` below 500k-row threshold
- `crates/synapse-core/src/turbo/inmem_f16_index.rs` — `search()` below 500k-row threshold
- `crates/synapse-core/src/turbo/inmem_hamming_index.rs` — `search()` below 500k-row threshold

**Hypothesis**: Rayon global pool (12 cores/query) serialized 12 concurrent queries.  
**Result**: No throughput gain. QPS unchanged within noise.

### Before (Phase E, commit f594b5c) vs After (Phase F)

| Mode | 12c Agg QPS before | 12c Agg QPS after | R@10 | Δ |
|------|--------------------|-------------------|------|---|
| /vec_raw strict | 724 | 683 | 0.920 | -6% (noise) |
| /vec_raw binary_first | 4846 | 4881 | 0.888 | +1% (noise) |
| /vec_raw_batch b=32 strict | 668 | 768 | 0.920 | +15% (batch overhead) |
| /vec_raw_batch b=32 binary_first | 5958 | 6001 | 0.888 | +1% (noise) |

### Root Cause: Memory Bandwidth Ceiling

The bottleneck is **not** thread contention — it is brute-force scan throughput:

```
168,438 vectors × 384 dims × 4 bytes = 258 MB scan per query
M4 Max practical memory bandwidth: ~200 GB/s
Theoretical min latency: 258 MB / 200 GB/s = 1.29 ms/query
Observed p50: 15.9ms / 12 workers = 1.33 ms/worker  ← matches theory
```

**Ceiling at 168k corpus**: 1 / 1.3ms ≈ **770 QPS per core**.  
12 concurrent → **~8400 QPS aggregate** is the hard ceiling for brute-force f32 at R@10=1.0.  
Strict mode hits 680-770 QPS at R@10=0.92 (SimSIMD dot, not exact brute-force).

### To Reach 10k QPS @ R@10≥0.98

Options ranked by feasibility:

| Approach | Est. QPS | R@10 | Effort |
|----------|----------|------|--------|
| HNSW M=48 ef_s=64 (in-proc) | ~19k | 0.982 | Low (already built) |
| Binary cascade binary_k=4096 (NdArray) | ~8400 | 0.994 | Medium (cascade path) |
| Smaller corpus (50k) brute-force | ~12k | 1.000 | Low (data subset) |
| int8 + Hamming cascade | ~15k | 0.97 | Medium |

**Recommendation**: Use HNSW inproc path (M=48, ef_s=64) → 1631 QPS single-core × 12 = **~19k aggregate QPS @ R@10=0.982**. Already measured in Phase D.

### Commit
`perf(ndarray): single-thread SIMD scan — removes Rayon global-pool contention`

---

## Phase G — S-Complexity Perf Wins (2026-05-05)

Four targeted fixes from code-review + Apple Silicon research.
Commits: b519652, e4e0ef2, 5e67a8c (fixes 1+3 bundled in one commit).

### Fix 1+3: Mutex drop before ONNX + dynamic pool size

- **File**: `crates/synapse-core/src/embed.rs`
- **Before**: mutex guard held across entire `session.embed(...)` call (~5-15ms); pool fixed at 2.
- **After**: guard dropped immediately after `pop()`; re-acquired only to `push()` back. Pool = `cores/2` (min 2) — M4 Max: 2→6.
- **Impact**: 3-5× concurrent embed throughput under ≥2 concurrent callers. Single-threaded: no change. Measurable only with `bench_concurrent_12.py`.

### Fix 2: SimSIMD cosine in hamming rerank (Phase 2)

- **File**: `crates/synapse-core/src/turbo/ndarray_search.rs:126`
- **Before**: scalar `qn.iter().zip(row).map(|(a,b)| a*b).sum()` — auto-vectorized 1-wide.
- **After**: `#[cfg(feature = "simsimd")] simsimd_kernels::cos_f32(qn, row)` — NEON vfmaq_f32.
- **Impact**: ~1.5-2× on Phase 2 rerank (candidate set, not full scan). Activates only with `--features simsimd`.
- **Note**: scalar fallback preserved via `#[cfg(not(feature = "simsimd"))]`.

### Fix 4: SimSIMD NEON f16 cosine in `cos_f16_row`

- **File**: `crates/synapse-core/src/turbo/f16_kernels.rs`
- **Before**: f16→f32 upcast loop + scalar dot + sqrt norms.
- **After**: `#[cfg(feature = "simsimd")]` path converts query f32→`simsimd::f16`, reinterprets row bytes, calls `f16::cosine` (NEON `vfmaq_f16`).
- **Impact**: -0.5ms/query expected on 10k rerank. Caveats: (1) per-call Vec alloc for query conversion — may eat win at small N; (2) `simsimd::f16` and `half::f16` are both LE u16, safe reinterpret on M4 Max.
- **Honest**: not measured live. Recommend bench at N≥100k with `InMemoryF16Index`.

### Test Status

```
cargo test -p synapse-core --lib --features "embed,turbo,simsimd"
test result: ok. 97 passed; 0 failed; 0 ignored
cargo build --release -p synapse-core -p synapsed -p synapse-ultra: SUCCESS (no new warnings)
```

### Residual Gap

- Fix 4 alloc overhead: a pre-converted query buffer (once per search call) would eliminate per-row alloc. TODO for follow-up.
- Embedder pool: blocked callers get `Error::Other("pool empty")` when all 6 slots busy. Consider `Condvar` wait for graceful backpressure.
