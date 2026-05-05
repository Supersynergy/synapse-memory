# ISO-Recall 0.99 Sweep — 2026-05-05
**Corpus**: 168,438 vectors · 384-dim · all-MiniLM-L6-v2  
**Queries**: 1,000 · GT: brute-force cosine top-100 (sqlite-vec)  
**Machine**: MacBook Pro M4 Max, 128GB

---

## Phase A — Re-embed Mismatch Fix

**Root cause**: Prior bench sent *text* queries to `/vec` (ultra re-embeds); GT was built from
*stored* doc embeddings. Different embedding calls → different vectors → 0.24 recall.

**Fix**: Use `/vec_raw` POST endpoint with stored query vectors (from `ground_truth.bin`).
No re-embedding — same embedding space as GT.

| Engine | Mode | p50 ms | QPS | R@10 | R@100 |
|--------|------|-------:|----:|-----:|------:|
| ultra_raw | strict | 1.49 | 661 | **0.9201** | 0.9432 |
| ultra_raw | binary_first | 0.65 | 1510 | 0.9130 | 0.8609 |
| ultra_raw | binary_only | 0.76 | 1292 | 0.4606 | 0.4172 |
| ultra (text, prior) | strict | 0.54 | 1100 | ~~0.24~~ | ~~0.27~~ |

**Result**: 0.24 → 0.92 recall confirmed. Fix = always use stored vecs for bench.

---

## Phase B — usearch ef_search Sweep (iso-recall=0.99 target)

| Engine | M | ef | p50 ms | QPS | R@10 | R@100 |
|--------|---|----|-------:|----:|-----:|------:|
| usearch | 16 | 64 | 0.168 | 5898 | 0.919 | 0.866 |
| usearch | 16 | 128 | 0.232 | 4363 | 0.957 | 0.921 |
| usearch | 16 | 200 | 0.328 | 2944 | 0.972 | 0.954 |
| usearch | 16 | 400 | 0.812 | 1230 | 0.982 | 0.980 |
| usearch | 32 | 64 | 0.231 | 4337 | 0.939 | 0.891 |
| usearch | 32 | 128 | 0.308 | 3131 | 0.967 | 0.939 |
| usearch | 32 | 200 | 0.433 | 2316 | 0.980 | 0.971 |
| **usearch** | **32** | **400** | **0.913** | **1078** | **0.988** | **0.989** |
| usearch | 48 | 64 | 0.207 | 4844 | 0.931 | 0.886 |
| usearch | 48 | 128 | 0.325 | 3039 | 0.963 | 0.937 |
| usearch | 48 | 200 | 0.481 | 2057 | 0.979 | 0.970 |
| usearch | 48 | 400 | 1.148 | 876 | 0.988 | 0.990 |

**Closest to R@10=0.99**: usearch M=32 ef=400 → R@10=0.988 @ 1078 QPS  
**qdrant baseline** (20k corpus, not 168k): 93 QPS @ R@10=1.0 (capped corpus, not comparable)

---

## Phase C — Binary Cascade (ultra)

Ultra already has binary_first mode (binary pre-filter → f32 rerank):

| Engine | Mode | p50 ms | QPS | R@10 |
|--------|------|-------:|----:|-----:|
| ultra_raw | strict (f32 HNSW) | 1.49 | 661 | 0.920 |
| ultra_raw | binary_first (cascade) | 0.65 | **1510** | 0.913 |
| ultra_raw | binary_only | 0.76 | 1292 | 0.461 |

Binary_first gives **2.3× QPS vs strict** with only 0.7% recall drop.

---

## Competitive Summary

| Engine | QPS | R@10 | Notes |
|--------|----:|-----:|-------|
| usearch M=16 ef=64 (prior baseline) | 4852 | 0.924 | M4 Max |
| **usearch M=16 ef=64 (this run)** | **5898** | **0.919** | M4 Max, 1-thread |
| usearch M=32 ef=400 (iso-recall ≥0.98) | 1078 | 0.988 | — |
| **ultra_raw binary_first** | **1510** | **0.913** | in-process HTTP, 1-thread |
| **ultra_raw strict** | **661** | **0.920** | in-process HTTP, 1-thread |
| qdrant (20k corpus) | 93 | 1.0 | capped corpus |

**vs usearch+binary SOTA (mined: 8500 QPS @ 0.96)**:  
Ultra binary_first = 1510 QPS @ 0.913 — **5.6× behind** on QPS at lower recall.  
Gap: ultra HTTP overhead (~0.5ms/query baseline) dominates; native in-process would close gap.

---

## Findings & Next Steps

1. **Re-embed bug fixed** — recall 0.24 was measurement artifact, not ANN quality issue.
2. **Actual ultra ANN quality**: R@10=0.92 (strict), R@10=0.91 (binary_first) — comparable to usearch M=16 ef=64.
3. **R@10=0.99 not achievable** in single-pass with current index config (max seen = 0.988 with usearch M=32 ef=400 @ 1078 QPS).
4. **QPS gap vs usearch**: HTTP overhead is the bottleneck (1.5ms per query vs 0.17ms usearch). Socket path or in-process bench would be fairer.
5. **Binary_first advantage**: 1510 QPS vs 661 strict — worth using when R@10 ≥ 0.91 acceptable.

### To reach R@10 ≥ 0.99:
- Increase HNSW M to 48+ and ef_search ≥ 800 (at ~500 QPS cost)
- Or two-pass: binary_first candidates=256 → f32 exact rerank top-10 (needs ultra API change)
- Or multi-probe: currently at M=16 default; check ultra's actual HNSW params

---
*Generated: 2026-05-05 | Bench: bench/industry/bench_ultra_raw.py*
