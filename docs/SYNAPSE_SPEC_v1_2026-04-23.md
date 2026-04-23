# Synapse SPEC v1 — 2026-04-23

**Authoritative spec** that defines what "best" means before we optimize. No measurement in this doc — only target-setting + architectural invariants. Measured verdict goes into `SYNAPSE_BEST_IN_CLASS_2026-04-23.md`.

## 1. Positioning

### Synapse IS

- **A single-file, pure-Rust, embedded agent memory** with BM25 + HNSW + KG + CRDT + Ed25519 in one binary
- **An embeddable library** (`synapse-core`) that any Rust program can link
- **A local-first store** — brain.db sits on disk, no server required
- **Signed & portable** via `.synx`/`.brainpack` format (Ed25519, reproducible)

### Synapse is NOT (anti-claims — do not market this)

- Not a distributed DB — no replication, no sharding across nodes
- Not a managed cloud service — no SaaS offering, no multi-tenant
- Not billion-scale — target ceiling is **100 M single-node**, plan projection only until PR-A2 IVF-PQ lands
- Not a LangChain-orchestrator — we store vectors and text, we do not call LLMs
- Not an LLM evaluation harness — scoring quality ≠ retrieval quality
- Not an Enterprise Search product — no ACLs, no OIDC, no query audit compliance

### Qoder context (what we know from repo signals + earlier sessions)

Qoder is a 2026 AI coding IDE that uses per-session memory + cross-session knowledge transfer. Research hits in this workspace (prior SuperKnow indexing, agent logs) show Qoder's memory-layer requirements read as: **fast cold-open** (users open IDE → instant memory), **small-batch put** (chat-like, not bulk), **hybrid BM25+vec query**, **per-project scope isolation**, **durable across crashes**. No source confirms IVF-PQ or 100 M. `uda ask "qoder …"` returned empty in this session — no local KB hits. This shapes SLO weight: cold-start + small-batch put + hybrid-query dominate.

## 2. Target Use-Cases (ranked)

1. **Claude-Code/IDE session memory** — per-session hot writes, per-project scope, telepathy read from other sessions
2. **Agent conversation memory** (chat + tool-call history, multi-turn)
3. **RAG for local repos** — 10k-1M code/doc chunks, hybrid BM25 + vec
4. **Offline / edge** — M-series laptop, no network, single file you can `scp`/`git`
5. **Signed knowledge distribution** — publish a `.brainpack`, consumer verifies Ed25519

Explicitly out-of-scope: 100 M+ analytics, billion-scale, multi-region, OLTP transaction workloads.

## 3. Hard Constraints (invariants)

Any PR that violates these is rejected by design.

- **Single-binary promise**: cargo-built `synapse` is one file. Sidecar files (e.g. `brain.db.usearch`) are permitted only when (a) created next to `brain.db`, (b) auto-rebuildable from `brain.db` if missing/corrupt, (c) the DB is still portable via `.brainpack` (which bundles them).
- **Pure Rust default-build**: `cargo build -p synapse-core` with default features must not pull C++ deps. usearch's C++ is gated behind `ann-usearch` (**default OFF**, opt-in with clear docs).
- **MIT/CC0 only** — no GPL, no BSL, no SaaS-only deps. Check: `cargo deny check licenses`.
- **No unwrap()/expect() in library code** — `grep -n 'unwrap()\|expect(' crates/synapse-core/src/*.rs` must return 0 in hot paths (tests exempt).
- **Public API is documented** — `#[deny(missing_docs)]` on `synapse-core` lib.rs (soft-target; current lib has gaps, tracked).
- **Public API additions are backwards-compatible** until 1.0; new methods, no rename/remove on existing signatures.
- **Every PR ships its bench-delta in the commit body** — before/after numbers, reproducible command. Broken rule = revert.

## 4. Performance SLOs (targets, not measurements)

Each SLO defines "met" when p95 on a fresh M4 Max 128 GB under the bench harness in `bench/bench_scale_ladder.py` stays below the number for ≥ 2 consecutive runs. Measured values live in the verdict doc.

### 4.1 Query latency p95 (ms) — vector kNN k=10, dim=384

| Scale | SLO (must) | Stretch (#1 mit Abstand) | Rationale |
|---|---:|---:|---|
| 1 k | ≤ 0.5 | ≤ 0.3 | IDE session-memory; users perceive <1 ms as "instant" |
| 10 k | ≤ 1.0 | ≤ 0.5 | Small project RAG |
| 100 k | ≤ 3.0 | ≤ 1.0 | Large repo RAG; beat Chroma's measured 0.76 ms |
| 1 M | ≤ 30 | ≤ 5 | Cross-repo knowledge; usearch HNSW log(N) target |
| 10 M | ≤ 100 | ≤ 20 | Stretch — honest PR-A2/B1 target, not today |

### 4.2 Ingest throughput — single process, put_batch 1000-batches

| Scale | SLO | Stretch |
|---|---:|---:|
| 10 k | ≤ 2 s wall | ≤ 0.5 s |
| 100 k | ≤ 15 s | ≤ 5 s |
| 1 M | ≤ 180 s (≈ 5.5 k docs/s) | ≤ 60 s (≈ 17 k/s) |
| 10 M | ≤ 1800 s | ≤ 600 s |

### 4.3 Cold-start p95

- Open `brain.db` + ready-for-query: ≤ **50 ms** SLO, ≤ 10 ms stretch (mmap).
- Library-mode `Store::open` (no daemon): ≤ 20 ms SLO.

### 4.4 Disk footprint

- ≤ 1.2 × sqlite-vec footprint at same N (for apples-to-apples)
- With int8 quant (PR-C1 later): target 0.25 × sqlite-vec

### 4.5 Recall quality

- `recall@10 vs brute-force ≥ 0.95` on synthetic sha-vector corpus (this session's measurement)
- `recall@10 vs GT ≥ 0.88` on MS-MARCO 100 k (PR-G1, not this session)

### 4.6 Crash recovery

- Process killed with `SIGKILL` mid-`put_batch`: `Store::open` succeeds, returns consistent state, no panics, no data-loss beyond last-uncommitted batch. SLO: **reopen ≤ 500 ms at 100 k**.

### 4.7 Concurrency

- 8 concurrent readers × 100 queries/s sustained: p95 regression ≤ 50 % vs solo (library-mode). SLO for PR-F1.

## 5. Feature Matrix

### Must-have for v0.3 (scale-100M cycle)

- `Store::open / put / put_batch / search / search_vec / search_hybrid / delete / stats`
- Feature-flagged `ann-usearch` with sidecar persistence + rebuild-on-corruption
- BLAKE3 embedding cache + batch-embed (shipped on `scale-100M-p0-embed-cache`)
- Ed25519 signing and verification
- Recall-parity harness that gates every PR-A* merge

### Nice-to-have for v0.4

- IVF-PQ index (PR-A2) for > 1 M regime
- int8 scalar quantization (PR-C1)
- WAL + crash-replay (PR-E1)
- Concurrent readers (PR-F1)
- MS-MARCO recall harness (PR-G1)

### Deferred

- Distributed query, replication, sharding across nodes
- Managed hosted tier
- Billion-scale
- Multilingual `bge-m3` / Arctic-v2 (planned but not in this scope)

## 6. Optimization Roster (derived from Chroma research + AUDIT 2026-04-20)

Ranked by effort/impact. Items 1-5 are today's target, 6-10 are the plan follow-ups.

| # | Opt | Effort | Expected on M4 Max | Status |
|:-:|---|---|---|---|
| 1 | `RUSTFLAGS="-C target-cpu=native"` + `lto=thin` + `codegen-units=1` | Trivial (Cargo.toml) | +5-20 % across the board | **ship this session** |
| 2 | `tikv-jemallocator` as global allocator | 3 lines | +2-4× alloc throughput under rayon | **ship this session** |
| 3 | `parking_lot` replace `std::Mutex` on hot paths | 30 min | +5-10 % on contested locks | already dep, verify usage |
| 4 | SQLite pragmas (`mmap_size`, `cache_size`) | 3 lines in `Store::open` | +10-30 % on read-heavy | **ship this session** — already 256 MB mmap; tune cache_size |
| 5 | usearch HNSW via `ann-usearch` + sidecar persist | 1-2 days | 100-400× at 100 k+ from micro-bench | **ship this session** as PR-A1-wire |
| 6 | `simsimd` for distance kernel (NEON auto) | 1 day | 3-10× on remaining brute-force paths | plan PR-C-fast |
| 7 | `roaring` bitmap prefilter (meta+vec) | 1 day | up to 10× on filtered queries | plan PR-C-filter |
| 8 | int8 scalar quantization (Matryoshka 256d) | 2 days | 4-12× disk, ~1 % recall loss | PR-C1 |
| 9 | Accelerate BLAS for batch embed | 2 days | 3-8× on ingest | PR-D3 |
| 10 | IVF-PQ (PR-A2) | 6-8 days | 100 M feasibility | plan critical |

## 7. Acceptance Gate (to merge any PR this session)

1. `cargo build --release -p <pkg> --all-features` green
2. `cargo test --release -p <pkg> --all-features` 100 % pass
3. `cargo clippy --release -p <pkg> --all-features -- -D warnings` 0 warnings (for new code paths)
4. Before/after criterion numbers in commit body
5. No `.unwrap()` / `.expect()` added to non-test code (manual `grep`)
6. For PR-A1-wire specifically: recall-parity harness shows ≥ 0.95 top-10 vs brute-force on 1000 queries

## 8. Honest Anti-Claims (MUST NOT be marketed)

- "Best agent-memory DB in the world" — we have no 100 M measurement and no multi-lingual recall data
- "Scales to billions" — no IVF-PQ yet, RAM-bound HNSW at 10 M
- "Faster than Chroma at every scale" — Chroma wins at 10 k and 100 k today (measured)
- "Recall parity with proprietary systems" — we have no MS-MARCO recall number yet

## 9. Defensible Claims (may be marketed once measurements exist)

- "On M4 Max, at 1 k agent-memory docs, Synapse is faster p95 than Chroma/LanceDB/Qdrant/sqlite-vec in the hybrid-search path." (measured)
- "Only single-file agent memory with BM25 + HNSW + KG + CRDT + Ed25519 in 1 MIT-licensed Rust binary." (architectural)
- "Sub-ms cold-open via mmap." (measured, in uc02/uc03)
- After PR-A1-wire: "At 100 k, Synapse library-mode p95 < X ms, competitive with Chroma." — only once X is measured here.

---
Author: hyperstack-heavy (Opus 4.7, Schritt 0 only). This spec is the contract; the verdict doc measures against it.
