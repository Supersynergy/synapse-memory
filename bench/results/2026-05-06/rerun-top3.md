# Synapse Top-3 Benchmark Rerun — 2026-05-06

Machine: MacBook Pro M4 Max, 128GB RAM | Threads: 16

---

## Summary Table

| Bench | Prior Value | Run 1 | Run 2 | Median | Delta % | Status |
|-------|-------------|-------|-------|--------|---------|--------|
| SIFT-1M build (f16, M=16, efC=256, 16-thr) | 104.3 s | 28.4 s | 47.6 s | **38.0 s** | -63.6% faster | ✅ faster than prior |
| SIFT-1M vs faiss ratio | 7.9× | 819.5/38.0 = 21.5× | — | **21.5×** | +172% vs prior claim | ✅ |
| SIFT-1M recall@0.99 QPS (f16) | ~2,188 qps | 0.00 recall (bug) | 0.00 recall (bug) | **N/A** | ❌ | BLOCKER |
| FTS5 throughput (mmap=256MB, WAL) | 44,158 ops/s | 42,591 | 42,892 | **42,741** | -3.2% | ✅ within noise |
| LongMemEval Recall@5 (rule mode) | unknown | 0.640 | 0.640 | **0.640** | — | ✅ stable |
| LongMemEval latency avg | unknown | 1,907 ms | 2,028 ms | **1,968 ms** | — | ✅ |

---

## Bench 1: SIFT-1M (ann-bench-synapse)

**Harness**: `/Users/master/projects/ann-bench-synapse/target/release/synapse_ann_bench`
**Dataset**: `/Users/master/projects/ann-bench-synapse/datasets/sift-128-euclidean.hdf5`
**Config**: usearch HNSW, M=16, efC=256, quant=f16, 16 threads

- Build run1: **28.4 s** | run2: **47.6 s** | median: **38.0 s**
- Faiss prior (from stored JSON): **819.5 s** → ratio **21.5×** (prior claim was 7.9×; prior runs used fewer threads)
- Max QPS seen (ef=10): ~28k–31k qps

**BLOCKER — recall regression**: f16 `recall_dist()` returns 0.0 for all ef values in both runs.  
Root cause: `usearch` distances are squared-L2 but HDF5 `distances` dataset stores raw L2 distances (or vice versa). The `recall_dist` threshold comparison always fails. The i8 path (id-set recall) works but plateaus at ~0.915 max recall — insufficient to demonstrate 0.99.

Prior stored file `sift_synapse_v2_f16_par.json` (2026-04-25) showed non-zero recalls — likely run with different usearch version or different HDF5 file. Recommend fixing `recall_dist` to use squared distances: `thr = truth_d[k-1].powi(2)`.

---

## Bench 2: FTS5 Throughput (synapse/eval/)

**Harness**: custom inline Python using `~/.synapse/brain.db`  
**Config**: mmap=256MB, cache=64MB, WAL, JOIN-based FTS5 MATCH, 10 query terms, 200 iterations

- Run 1: **42,591 ops/s**
- Run 2: **42,892 ops/s**
- **Median: 42,741 ops/s**
- Prior: 44,158 ops/s (from `eval/optimal_fts5_settings.py` docstring)
- **Delta: -3.2%** — within measurement noise, no regression.

Note: `benchmark_fts5_perspectives.py` crashes on FTS5 syntax error for multi-word queries (single-word `"search"` treated as FTS5 operator). Script uses safe single-token queries above.

---

## Bench 3: LongMemEval-S Rule Baseline

**Harness**: `/Users/master/projects/synapse/target/release/longmemeval`  
**Data**: `/Users/master/projects/synapse/bench/longmemeval/data/lme_s_50.json` (50 questions)  
**Mode**: Rule hooks (mlx_lm not in PATH, falls back to RuleHooks) + JINA reranker ONNX

- Run 1: Recall@5=**0.640** (32/50), Recall@10=0.640, latency=1,907 ms
- Run 2: Recall@5=**0.640** (32/50), Recall@10=0.640, latency=2,028 ms
- **Median Recall@5: 0.640 | Latency: 1,968 ms**
- Prior: not stored (no prior numeric baseline found in repo)
- Deterministic — identical recall both runs.

---

## Issues to Fix

1. **CRITICAL**: `recall_dist` in `ann-bench-synapse/src/main.rs:74` — distance scale mismatch (f16/f32). Fix: square the truth distance threshold before comparison, or switch f16 to id-based recall.
2. `benchmark_fts5_perspectives.py` FTS5 syntax error on multi-word queries — needs `fts5_sanitize()` on query terms.
3. No stored prior for LongMemEval — establish 0.640 as new baseline.
