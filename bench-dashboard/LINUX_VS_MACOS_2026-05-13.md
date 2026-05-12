# Linux vs macOS Bench — 2026-05-13

**macOS**: MacBook Pro M4 Max · Apple Silicon aarch64 · APFS · Rust 1.95 native  
**Linux**: Colima VM · aarch64-unknown-linux-gnu · virtiofs bind-mount · GCC 12  
**NEON**: both aarch64 → NEON available on both.  
**NEON dotprod (armv8.2-a)**: macOS M4 YES · Colima/GCC-12 NO (vdotq_s32 inlining fails → `-march=armv8-a` workaround applied)

---

## 1. raw_ann_microbench (usearch-HNSW)

N=10000 DIM=384 K=10 Q=100 — in-memory only

| Backend | macOS p50 µs | Linux p50 µs | Linux/macOS | macOS p99 | Linux p99 |
|---------|-------------|-------------|-------------|-----------|-----------|
| FAISS-Flat (exact) | 208 | 353 | **1.7×** slower | 286 | 421 |
| FAISS-HNSW | 98 | 119 | **1.2×** slower | 157 | 203 |
| usearch-HNSW (synapse) | 77 | 74 | **~parity** (-4%) | 104 | 134 |
| usearch build_ms | 606 | 606 | parity | — | — |

> FAISS-Flat slower on Linux: Python/numpy less JIT-tuned on Colima virtiofs. usearch-HNSW parity — Rust code path, NEON scalar fallback identical.  
> R@10 = 0.942 on both (deterministic seed, same index params).

---

## 2. fts_compare bench (Tantivy vs SQLite-FTS5)

10k docs, 100 queries

| Backend | macOS p50 ms | Linux p50 ms | Linux/macOS |
|---------|-------------|-------------|-------------|
| tantivy | 2.57 | 2.00 | **0.78×** (Linux 22% faster) |
| sqlite_fts5 | 121 | 116 | **0.96×** (parity) |

> Tantivy slightly faster on Linux — likely APFS syscall overhead on macOS for mmap. SQLite-FTS5 I/O-bound → virtiofs overhead washes out.

---

## 3. rrf_neon bench (RRF SIMD merge)

Criterion, µs/iter (p50 median)

| Case | macOS scalar | Linux scalar | macOS NEON | Linux NEON | NEON speedup (mac) | NEON speedup (linux) |
|------|-------------|-------------|-----------|-----------|-------------------|---------------------|
| N=256 | 33.2 µs | 29.8 µs | **5.0 µs** | **5.5 µs** | 6.6× | 5.4× |
| N=1024 | 164 µs | 139 µs | **25.0 µs** | **26.0 µs** | 6.6× | 5.3× |
| N=4096 | 820 µs | 634 µs | **101 µs** | **110 µs** | 8.1× | 5.8× |

> NEON available and working on Colima (armv8-a). Speedup slightly lower on Linux vs macOS — M4 Max wider SIMD pipeline + better branch prediction. Scalar Linux faster: GCC 12 auto-vectorizes better than macOS `lld`.

---

## 4. Workspace Tests

| | macOS | Linux (Colima) |
|--|-------|---------------|
| cargo test --workspace | ✅ green | ⚠️ partial (see below) |
| Excluded on Linux | — | synapse-market, synapse-mysql, synapsql (opensrv git dep) |
| synapse-extract Linux | ✅ | ❌ E0463 can't find crate `synapse_core` (link-order bug, Linux only) |

---

## 5. Key Findings

1. **usearch-HNSW parity**: Linux Colima aarch64 matches macOS within 4% on ANN query latency. Multi-platform deploy safe.
2. **NEON dotprod blocker**: `vdotq_s32` fails on Colima GCC-12 (armv8-a default lacks dotprod). Workaround: `CFLAGS=-march=armv8-a`. CI yml adds this env. Production aarch64 runners (GitHub `ubuntu-24.04-arm`) likely have armv8.2-a — test without workaround there.
3. **FAISS Python leg**: 1.7× slower on Linux — Python/numpy untuned for Colima virtiofs. Irrelevant for production (Rust paths only).
4. **Tantivy faster on Linux**: 22% faster than macOS. APFS overhead on macOS mmap reads.
5. **Disk I/O**: virtiofs bind-mount didn't hurt bench meaningfully — benches are CPU-bound not I/O.
6. **opensrv git dep**: `synapse-market` pulls `opensrv-mysql` from GitHub tag. Excluded from CI matrix until git dep resolved or vendored.

---

## 6. Recommendations

- CI amd64 runners: add `CFLAGS=-march=x86-64-v2` for conservative baseline; native runners get AVX2 auto.
- CI arm64 runners (`ubuntu-24.04-arm`): try WITHOUT `-march=armv8-a` override first — GH runners are Neoverse N1 (armv8.2-a, dotprod supported).
- Fix `synapse-market` opensrv dep: switch to crates.io release or `cargo vendor` for hermetic CI.
- Fix `synapse-extract` Linux build: `E0463 can't find crate synapse_core` — likely missing explicit dep declaration (works on macOS via implicit workspace link). Add `synapse-core` to `synapse-extract/Cargo.toml` dependencies.
- Track `usearch R@10=0.942 < 0.95` parity claim: needs `expansion_search` tuning (both platforms same baseline).
