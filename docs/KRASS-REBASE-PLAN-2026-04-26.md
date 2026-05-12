# KRASS REBASE PLAN — Synapse 2026-04-26

> Synthesis of SPEED-INTEL-FINAL, SPEED-INTEL-MASTER, MASTER-CHECK, SYNERGY, RESULTS_FAST,
> PIONEER, MASTERPLAN + grepgod scans of `~/projects/synapse/crates/` + superml ranking.
> All numbers traceable. M4 Max baseline. (REBASE-INTEL & MOMENTUM docs not yet written —
> placeholders below; refresh once parallel agents land.)

---

## Section 1 — Where Synapse Loses Today

Source: `bench/comprehensive/RESULTS_FAST.md` + `MASTER-CHECK-2026-04-25.md` + `SYNERGY` §3.

| Phase | Workload | Synapse | Winner | Gap | Honest verdict |
|-------|----------|--------:|--------|----:|----------------|
| **A** Bulk insert (10k, w/ embed) | docs/s | 47,050 | sqlite-vec (~tied) | — | parity, ceiling = embed in critical path |
| **C** Mixed 80/20 OLTP | ops/s | 283.6 | libSQL BEGIN CONCURRENT path | **4×** | write-mix collapses, no concurrent txn |
| **D** 8-thread point-select | ops/s | 46,981 | sqlite-vec 55,943 | 1.2× | RwLock<BTreeMap> read contention |
| **E** Recall@10 | recall | 0.096 | sqlite-vec 0.112 | **broken** | non-publishable until ≥0.85 |
| **F** Concurrency sweep | QPS | 1,078 (sysbench TLS) | MariaDB 7,626 | **7×** | no pool, no shard-per-core |
| **G** Parquet pre-embed bulk | docs/s | 27,852 | (tied with sqlite-vec) | — | no native mmap path; 100k/s unreached |
| **H** Group-commit batch sweep | — | flat after batch=1k | FastFS-style coalesce | 3-5× | no async group commit |
| **I** Cache hitrate | — | n/a | LRU embed+query | — | not implemented |
| **J** Embed compute | — | fastembed CPU | MLX Metal batch | 10× | embed dominates at <10k batch |

**Headline losses:** F (7× MariaDB), C (4× libSQL), J (10× MLX), E (recall broken).

---

## Section 2 — The Krass Stack (Rebased)

Sources: SPEED-INTEL-FINAL Speed-A/B/C, PIONEER P0/P1, SYNERGY phase-map.

| Layer | Current | Target | Rust crates | Validates phase | Source claim |
|-------|---------|--------|-------------|-----------------|--------------|
| **Storage WAL** | rusqlite single-conn | deadpool_sqlite + async group-commit (FastFS) | `deadpool-sqlite`, `tokio` | A, H | Speed-A #2 (3-5×) |
| **Storage txn** | SQLite WAL | libSQL `BEGIN CONCURRENT` | `libsql` | C | Synergy #4 (4× write-mix) |
| **Concurrency** | `RwLock<BTreeMap>` | `dashmap` v5 + `parking_lot` | `dashmap`, `parking_lot` | D, F | Speed-A #1 (4× reads) |
| **Read path** | shared tokio runtime | shard-per-core via `tokio::task_local` | `tokio` (per-core arenas) | D, F | Speed-A #3 (2-3× p99) |
| **Pool** | none | `deadpool` 64 conns | `deadpool` | F | Synergy #11 (3×) |
| **Vector index** | sqlite-vec brute | HNSW + IVF-PQ + roaring filter | `hnsw_rs`, `instant-distance`, `roaring` | E, F | PIONEER P1 (recall fix) |
| **Compute / SIMD** | numpy/fastembed | `simsimd` RRF + Metal | `simsimd`, `candle-core` | D, E | Synergy #8, J |
| **Embed** | fastembed CPU BGE-small | MLX Metal batch 256 | `mlx-rs` (FFI) or `candle` Metal | G, J | Speed-B Tier-2 (10×) + PIONEER P0 |
| **Embed dedup** | none | BLAKE3 hash-cache | `blake3` | A | PIONEER P0 (100× dup) |
| **Cache** | none | LRU embed + query | `moka` | I | Synergy #9 (2×) |
| **Bulk path** | row-by-row + embed | Parquet → mmap → batch insert | `arrow2`/`parquet`, `memmap2` | G | Speed-B Tier-2 (1M/s) |
| **FTS write** | per-row INSERT | multi-row INSERT VALUES batch | rusqlite | A, H | Synergy #5 (3-5×) |
| **Serving** | python asyncio | `axum` + `tower` + group-commit middleware | `axum`, `hyper`, `tower` | C, D, H | Speed-C |

> Note: `fjall` / `rkyv` from the brief are reserved for a Tier-3 (>10M docs/s, 60d)
> redb-sidecar branch — NOT day-1. SQLite remains the truth-store; this preserves
> MASTERPLAN §11 (no-go: drop SQLite).

---

## Section 3 — 30 / 60 / 90 Day Roadmap

superml ranking (effort×gain×risk inverted, normalized; effort in days, gain = bench-validated):

| Rank | Change | Effort | Gain | Risk | Score |
|-----:|--------|-------:|-----:|------|------:|
| 1 | dashmap replaces RwLock | 3d | 4× | low | **9.6** |
| 2 | Async group commit | 5d | 3-5× | med | 9.0 |
| 3 | BLAKE3 embed dedup | 2d | 100× (dup case) | low | 8.8 |
| 4 | deadpool 64 conn pool | 2d | 3× | low | 8.5 |
| 5 | FTS5 multi-row INSERT | 3d | 3-5× | low | 8.4 |
| 6 | LRU moka cache | 2d | 2× | low | 8.0 |
| 7 | simsimd SIMD RRF | 4d | latency | low | 7.6 |
| 8 | shard-per-core read | 7d | 2-3× p99 | med | 7.2 |
| 9 | libSQL BEGIN CONCURRENT | 7d | 4× write-mix | med | 7.0 |
| 10 | Parquet bulk path | 10d | 10-20× ingest | med | 6.8 |
| 11 | HNSW + IVF-PQ | 10d | recall+latency | high | 5.4 |
| 12 | MLX Metal embed | 14d | 10× embed | high | 5.0 |

### Day 1-30 — Foundation Sprint (validate Phase A, F, D, H, I)
- W1: flamegraph + dashmap (#1) + deadpool (#4) + BLAKE3 (#3) → bench D, F
- W2: async group commit (#2) + FTS5 multi-row (#5) + moka LRU (#6) → bench A, H, I
- W3: simsimd RRF (#7) + libSQL BEGIN CONCURRENT (#9 start) → bench C
- W4: shard-per-core (#8) → bench D, F regen

**Gate metrics** (vs RESULTS_FAST baseline):
- D 8t: 47k → **180k ops/s** (4× target)
- F sysbench TLS: 1,078 → **15-25k QPS** (pool+dashmap step)
- A bulk: 47k → **140-235k docs/s** (group-commit + FTS5 batch)
- I cache: n/a → **≥70% hitrate** repeat
- C mixed: 284 → **1,100+ ops/s** (libSQL concurrent)

### Day 31-60 — Index Rebase (validate Phase E, F)
- HNSW (`hnsw_rs`) + IVF-PQ + roaring tag-filter
- Recall fix: rerank with simsimd, alpha-tuned RRF
- Roaring bitmap pre-filter on metadata before vec scan

**Gate metrics:**
- E recall@10: 0.096 → **≥0.90**
- F mixed vec+filter: undefined → **5-15k QPS** sustained

### Day 61-90 — Compute Layer (validate Phase J, G)
- MLX Metal embedder (M-series) — `mlx-rs` FFI or candle-Metal fallback
- Parquet pre-embed bulk path with `memmap2` zero-copy
- Bench J: embed throughput; bench G: 1M docs/s pre-embedded ingest

**Gate metrics:**
- J: fastembed CPU → **10× MLX Metal batch=256**
- G: 27,852 → **470k-1M docs/s** (pre-embed Parquet)
- Headline: **50k QPS sustained @ 8t + 1M docs/s pre-embedded** (matches SPEED-INTEL-FINAL honest revised target)

---

## Section 4 — Risk Map

| Change | Blast radius | Rollback | Dep risks |
|--------|--------------|----------|-----------|
| dashmap | concurrent index only | revert commit; keep RwLock path behind feature flag | none — drop-in API |
| async group commit | write path; durability semantics | per-tenant `sync=full` override; bench H gate | fsync timing — must doc consistency model |
| BLAKE3 dedup | put path | dedup off via env `SYN_DEDUP=0` | hash collision negligible |
| deadpool pool | conn lifecycle | env `SYN_POOL=1` | tokio runtime mix |
| libSQL BEGIN CONCURRENT | swap rusqlite→libsql | feature-gate `libsql-backend` | sqlite-vec ABI compat — must verify auto_extension load |
| HNSW/IVF-PQ | replaces sqlite-vec for >100k rows | hybrid: sqlite-vec ≤100k, HNSW above | rebuild cost on cold start; index versioning |
| MLX embed | M-series only | fallback fastembed CPU on non-Apple | mlx-rs maturity — stay behind feature flag |
| shard-per-core | tokio runtime topology | runtime flag | task-local pinning portability |
| Parquet bulk | new ingest path | parallel CLI cmd, doesn't touch single-doc PUT | arrow2/parquet version churn |

**Cross-cutting:** recall@10=0.096 must be fixed BEFORE any speed claim is published
(SYNERGY §3). Phase E gate is non-negotiable.

---

## Section 5 — One-Pager Pitch

**Why this rebase:** Synapse is already memory-optimal at the index layer (23µs query,
sub-ms hybrid) but loses 7× to MariaDB on concurrent OLTP and 10× to MLX on embed.
The 12-change Krass stack — dashmap, async group commit, BLAKE3 dedup, deadpool,
FTS5 batch, moka, simsimd, shard-per-core, libSQL BEGIN CONCURRENT, HNSW/IVF-PQ,
Parquet bulk path, MLX Metal — closes every measured gap without abandoning the
single-file SQLite truth-store. Each change has an existing crate, a bench phase
that gates it, and a rollback. Compound projection (Speed-A): 1,078 → 50k QPS @8t
in 14d; 1M docs/s pre-embedded ingest in 14d more.

**What Synapse becomes:** the only embedded memory layer that ships SQL + FTS5 +
HNSW vec + AI built-ins + MCP daemon in 26MB, with Apple Silicon Metal embedding
and MariaDB-class concurrent OLTP — at sub-ms hybrid query latency. Leadership
claim after Day 90: *"AI-era embedded DB beats MySQL on bulk 100×, matches on OLTP
8t, ships everything Postgres + Qdrant + fastembed do — in one file."* Not "10×
faster MySQL" (gets debunked at scale); the win is the COMPOUND CAPABILITY no
existing system in any language has shipped.

---

## Appendix — grepgod scan summary
- `TODO|FIXME|XXX|PERF` in `crates/`: noisy hits (mostly vendored deps in
  `universal2adapter/`); core Synapse crates are TODO-light. Targeted scan needed
  on `crates/synapse-core/src/` only — current pass returned vendored capnproto/etc.
- `unwrap()` in `synapse-core/src/`: hits dominated by vendored test files; need
  `--no-vcs --glob '!third-party'` rerun before triage.
- `Vec::new()` in `synapse-core/src/`: **0 matches** in scan window — allocation
  hotspots already minimal or factored.
- `tokio::sync::Mutex|RwLock`: matches confirm Speed-A #1 candidate set;
  primary targets are concurrent-index sites (dashmap migration).

**Action:** rerun grepgod scoped to `crates/synapse-{core,daemon,turbo}/src/` only,
excluding `third-party/` and `vendor/`. Add to Day-1 W1 task list.
