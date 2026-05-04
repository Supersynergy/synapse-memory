# Synapse Rebase Intel — 2026-04-26
**Mission:** overtake LanceDB, Qdrant, Milvus, Chroma, sqlite-vec.
**Method:** ghgrep mining of production Rust vector/memory DBs + arch synthesis.
**Current synapse-core stack:** rusqlite + sqlite-vec 0.1 + redb + fastembed + ndarray + linfa.
**Verdict:** sqlite-vec is the bottleneck (linear scan, no PQ, no mmap, no SIMD-dispatch). Path to dominance below.

---

## Section 1 — Top 10 Architectural Patterns Beating Current Synapse

| # | Pattern | Best-in-class repo (evidence) | LOC effort | Bench gain | Risk |
|---|---------|------------------------------|-----------:|-----------:|-----|
| 1 | **SimSIMD distance dispatch** (NEON/AVX-512/SVE runtime pick) | [unum-cloud/USearch](https://github.com/unum-cloud/USearch), [duckdb/duckdb-vss](https://github.com/duckdb/duckdb-vss), [chroma-core/chroma](https://github.com/chroma-core/chroma) `rust/index/src/quantization/multi_bit.rs` | ~150 | 5–25× cosine/dot vs naive | low (drop-in crate) |
| 2 | **HNSW with deletion + epoch GC** | [nnethercott/hannoy](https://github.com/nnethercott/hannoy) (benches/speed.rs), [Rika-Labs/sgrep](https://github.com/Rika-Labs/sgrep) `src/search/hnsw.rs`, qdrant `segment` | ~2k | 50–100× recall@10 vs flat | medium (concurrency) |
| 3 | **IVF-PQ + OPQ rotation** (Faiss-style coarse+fine) | [chroma-core/chroma](https://github.com/chroma-core/chroma) `rust/index/src/quantization/`, lancedb/lance | ~3k | 16–32× memory shrink, 4× QPS at 100M | medium |
| 4 | **memmap2 + rkyv zero-copy posting lists** | [ruvnet/RuVector](https://github.com/ruvnet/RuVector) `docs/postgres/postgres-zero-copy-memory.md`, qdrant segment loaders | ~400 | 10× cold-start, 0-alloc reads | low |
| 5 | **fjall LSM as KV/metadata store** (replaces redb for hot writes) | [fjall-rs/fjall](https://github.com/fjall-rs/fjall) (21 examples), [marvin-j97/smoltable](https://github.com/marvin-j97/smoltable) production | ~300 | 3–5× write tput vs redb on bursty | low (mature 2026) |
| 6 | **RoaringBitmap pre-filter** for tag/payload ANN | qdrant `payload_storage`, [daviddrummond95/polar_llama](https://github.com/daviddrummond95/polar_llama) `src/ann.rs` | ~250 | 100× on filtered queries | low |
| 7 | **WAL group-commit + fsync coalescing** | fjall WAL, sled, surrealkv | ~500 | 10–50× small-write throughput | medium (durability bugs) |
| 8 | **DashMap shard + tokio::task_local for hot index cache** | qdrant `id_tracker`, lancedb cache | ~150 | 3× concurrent QPS | low |
| 9 | **Multi-bit quantization (1-bit/2-bit binary)** | [chroma-core/chroma](https://github.com/chroma-core/chroma) `rust/index/src/quantization/multi_bit.rs` | ~600 | 32× mem, 8× QPS, recall@10≈0.95 | medium |
| 10 | **Async io_uring/glommio segment loader** (Linux) + mmap fallback (mac) | scylladb seastar, glommio storage examples | ~700 | 2–4× scan tput on NVMe | high (linux-only path) |

---

## Section 2 — Rust Crates to Add to synapse-core

| Crate | Version | Role | Replaces | Bench claim |
|-------|---------|------|----------|-------------|
| `simsimd` | 6.x | Runtime SIMD distance (cos/dot/L2/hamming) | hand-written ndarray loops | 5–25× (USearch bench) |
| `fjall` | 2.x | LSM KV for WAL+metadata+payload | redb (writes) / sled | 3–5× write, 21 prod examples |
| `memmap2` | 0.9 | mmap segments | full-load Vec<u8> | 10× cold-start |
| `rkyv` | 0.8 | Zero-copy archived structs (HNSW nodes, posting lists) | bincode/postcard | 0-alloc deserialize |
| `roaring` | 0.10 | Bitmap pre-filter | HashSet<u64> | 10–100× on filtered ANN |
| `hnsw_rs` or fork of `hannoy` | latest | Pure Rust HNSW with delete | sqlite-vec linear scan | 50–100× @ 1M vecs |
| `dashmap` | 6.x | Sharded concurrent map | RwLock<HashMap> | 3× concurrent reads |
| `crossbeam-epoch` | 0.9 | Epoch-based reclamation for HNSW deletes | Arc churn | unblocks lock-free reads |
| `bytemuck` | 1.x | POD casts for embeddings | unsafe transmute | safety + zero-cost |
| `glommio` (linux) / `tokio-uring` | latest | io_uring runtime, behind `cfg(linux)` | tokio fs | 2–4× scan |
| `rayon` | 1.10 | Parallel batch insert / build | manual threads | trivial scale-out |
| `mimalloc` | 0.1 | Global allocator | system malloc | 10–20% on alloc-heavy |
| `zerocopy` | 0.8 | Layout-checked casts | `unsafe` | safety net |

Drop or demote: `linfa-clustering` (heavy, swap for hand-rolled k-means on simsimd), `ndarray` (keep only for ML training paths, not hot ANN).

---

## Section 3 — Antipatterns to Avoid

1. **SurrealDB / sled as primary KV** — sled abandoned 2024, surreal layered storage has fsync amplification. fjall is the 2026 answer.
2. **Pure-Python wrapper around C++ index** (Chroma <0.5, Milvus) — every QPS goes through GIL/FFI. Synapse already wins by being Rust-end-to-end; do not re-introduce a Python data-path.
3. **One giant locked HNSW** — qdrant's pre-segmentation lesson. Use **N segments + parallel search + heap-merge**. Single-graph repos (early hora, instant-distance) plateau at ~5k QPS.
4. **bincode/postcard for hot indexes** — deserializing 10M nodes per restart kills cold-start. Use rkyv or raw mmap.
5. **JSON payloads in row** — qdrant lesson. Store payload in fjall keyed by point-id, project only filtered cols into roaring bitmaps.
6. **Re-implementing FAISS in Rust from scratch** — chroma's `rust/index` shows: lift quantization math from FAISS C++ (BSD), port only the loop.
7. **Ignoring NEON on Apple Silicon** — naive Rust loops leave 8× perf on M-series. simsimd handles it.
8. **Async everywhere** — HNSW search is CPU-bound. Use `spawn_blocking` + rayon, not tokio inside the inner loop (qdrant's `tokio-uring` regression 2024).
9. **Locking the whole WAL on commit** — group-commit pattern (fjall, postgres) batches fsyncs across writers.
10. **Rebuilding index on every insert** — incremental HNSW + tombstone + nightly compact (lance pattern).

---

## Section 4 — "Synapse Krass" Architecture (3-Stack Synthesis)

```
                       ┌──────────────────────────────────────────┐
                       │    axum :7777  (REST + gRPC tonic)       │
                       │    + tower rate-limit + tracing-otel     │
                       └────────────────┬─────────────────────────┘
                                        │
                  ┌─────────────────────┴─────────────────────┐
                  │     Query Planner (cost-based)            │
                  │  filter? → roaring → ANN → rerank         │
                  └─────────┬───────────────────────┬─────────┘
                            │                       │
                ┌───────────▼─────────┐   ┌─────────▼──────────┐
                │  COMPUTE LAYER       │   │  WRITE LAYER       │
                │  ─────────────────   │   │  ─────────────────  │
                │  simsimd dispatch    │   │  WAL group-commit  │
                │   NEON/AVX/SVE       │   │   (16ms window)    │
                │  rayon batch search  │   │  fjall Keyspace    │
                │  MLX bridge (mac)    │   │   ├─ partitions    │
                │  dashmap hot cache   │   │   │  • vectors     │
                └───────────┬─────────┘   │   │  • payload     │
                            │              │   │  • tombstones  │
                            │              │   └─ SSI tx        │
                            │              └─────────┬──────────┘
                ┌───────────▼──────────────────────────▼──────────┐
                │             INDEX LAYER (sharded segments)      │
                │  ──────────────────────────────────────────────  │
                │  HNSW (hannoy-fork) + epoch-GC delete            │
                │  IVF-PQ (chroma-port) for >10M segments          │
                │  Multi-bit binary index for filter-heavy         │
                │  RoaringBitmap payload pre-filter                │
                │  rkyv archived nodes  ◄── memmap2 segment files  │
                └───────────┬──────────────────────────────────────┘
                            │
                ┌───────────▼──────────────────────────────────────┐
                │             STORAGE LAYER                        │
                │  ──────────────────────────────────────────────  │
                │  segments/*.synseg  (mmap, rkyv-archived)        │
                │  fjall/  (LSM: hot writes, metadata, WAL)        │
                │  cold/   (zstd-compacted, lance-format optional) │
                │  io_uring (linux) │ mmap (mac)   ── cfg-gated    │
                └──────────────────────────────────────────────────┘

Concurrency:  tokio (IO) ║ rayon (CPU) ║ crossbeam-epoch (lock-free reads)
              dashmap shards = num_cpus * 4
Allocator:    mimalloc global
Build:        sccache + mold (linux) / lld (mac)
```

### Sharding/segment model (the qdrant lesson + lance lesson)
- 1 segment = 1 mmap file (`*.synseg`) = rkyv-archived `{header, hnsw_graph, vectors, payload_offsets}`.
- New writes → fjall WAL → in-memory mutable segment → flush at 64MB or 60s → seal → mmap.
- Background compactor merges N small → 1 big (lance-style), rebuilds HNSW with rayon.
- Search: parallel `rayon::join` across segments → bounded heap merge.

### Migration path (4 phases)
1. **P1 (1 wk):** drop simsimd + dashmap + roaring. Replace sqlite-vec scan with simsimd flat. **Expected 5–10×.**
2. **P2 (2 wk):** add HNSW segment via hannoy-fork; rkyv+memmap2 segment file format. **50× @ 1M.**
3. **P3 (3 wk):** swap redb→fjall for WAL/payload; group-commit. **10× write.**
4. **P4 (4 wk):** IVF-PQ + multi-bit for >10M; io_uring linux path. **>100M parity with lance/qdrant.**

### Bench targets (after P3)
- 1M × 768d cosine: **>50k QPS single node** (qdrant ~30k, chroma ~15k, sqlite-vec ~2k)
- Insert: **>200k vec/s** with WAL durability (fjall-bench territory)
- Cold start 10M index: **<2s** (mmap+rkyv) vs qdrant ~30s
- Memory @ 10M × 768d w/ PQ: **<1.5GB** (vs raw 30GB)

---

## Evidence Repos (deepest learning targets)
- **chroma-core/chroma** `rust/index/` — multi-bit quantization, segment management, the Rust-native rewrite playbook.
- **fjall-rs/fjall** — 21 production examples; SSI tx, secondary index, triplestore patterns directly applicable.
- **nnethercott/hannoy** — modern Rust HNSW with deletes + benches.
- **ruvnet/RuVector** — zero-copy + simsimd integration in Rust workspace shape similar to synapse.
- **unum-cloud/USearch** — simsimd canonical user; copy their dispatch table.
- **marvin-j97/rust-storage-bench** — fjall vs redb vs sled vs rocksdb numbers; use their harness.
- **Cuprate/cuprate** `storage/blockchain/` — fjall in adversarial-load production (Monero node).

## Anti-evidence
- "beats qdrant" / "beats lancedb" / "million qps" ghgrep queries returned essentially zero credible Rust repos — meaning **the throne is unclaimed in 2026**. Ship P1+P2 and synapse can credibly claim the bench.
