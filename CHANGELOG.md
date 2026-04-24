# Changelog

## v2.1-m4max-preview — 2026-04-24 · M4 Max turbo layer (final state)

**SIMSIMD-accelerated kernels, Matryoshka / Binary-Matryoshka / f16 storage,
unified `TextEmbedder` trait, `AdaptiveRouter` Thompson bandit, `MultiIndex`
one-liner, 11 real-world bench loaders, LangChain / Mem0 / LlamaIndex
adapters, 8 pytest smoke cases, audit-swept by 4 parallel Haiku subagents —
0 errors · 0 warnings · 68 tests pass · clippy clean.** Every module
feature-gated; pure-Rust default build unchanged. Apple-Silicon-first.

### 24 iterations shipped

| Iter | Output |
|------|--------|
| 1–4  | SimSIMD + Matryoshka + `TextEmbedder` trait + bench progression |
| 5–10 | MRL decorator · Ollama batch fix · `AdaptiveRouter` · synapse-py · adapters |
| 11–14| pyproject + pytest · InMemoryI8/Hamming + rerank · 50-usecase bench suite |
| 15–18| Binary-MRL · rayon chunk-tuning · f16 helpers · InMemoryF16Index |
| 19–21| `PyF16Index` · `MultiIndex` one-liner · SPEC-Sync |
| 22–24| 9-format loader · brainpack scaffold · audit-sweep · CHANGELOG close |

### Ecosystem snapshot

- Rust crates touched: `synapse-core` + new `synapse-py`
- Rust tests: **68 passing · 0 failing · 0 warnings** (release)
- pytest cases: **8** (MultiIndex, F16, rerank pipeline, AdaptiveRouter, Brain, kernels)
- Bench harnesses: **12 real-world** (Obsidian, ChatGPT, Gmail, Slack, Apple Notes, Logseq, iMessage, WhatsApp, Linear, Photo-CLIP, Health-fuse + generic)
- File formats supported in harness: **.md .txt .rst .markdown .org .json .jsonl .ndjson .csv .tsv .db .sqlite .sqlite3 .syn .synx .synapse .brainpack**
- Framework adapters: **LangChain · Mem0 · LlamaIndex** (all under `synapse-py/examples/`)

### Measured (M4 Max, 100 k × 384, cold)

| kernel | µs/q | speed-up | note |
|---|---:|---:|---|
| S0 scalar cos f32 | 13 210 | 1.00× | baseline |
| S3 SimSIMD int8 | 287 | 46× | prod-ready |
| S4 SimSIMD 1-bit | **192** | **71×** | peak raw speed |
| S5 MRL-128 | 375 | 35× | |
| S7 Hamming→i8 k10 | 425 | 31× | full-recall pipeline |
| S8 f16 storage | 5 141 | 4× | 50 % RAM |

### Audit findings (parallel Haiku-agent sweep)

- Bottleneck: `MultiIndex::build` 3× clone — documented, future single-pass fused builder.
- Best-practice: Mutex poison recovery added; `#[must_use]` on `search`.
- Error-hunt: **zero errors or warnings across all feature permutations**.

### Task coverage

- [x] #2 1-bit Hamming kernel
- [x] #3 Unix-socket RPC server
- [x] #4 LangChain + Mem0 + LlamaIndex adapters
- [x] #5 Full bench vs faiss + fastembed
- [x] #6 Brain class — end-to-end memory store
- [x] #7 Tests + CI-ready smoke
- [ ] #1 Custom Metal shader via `objc2-metal` — deferred, needs dedicated session

### Rollback

- `backup-pre-m4max-bench-2026-04-24` git tag
- `~/projects/data/synapse-backup-2026-04-24.tgz` (75 MB)

## v2.1-m4max-preview — 2026-04-24 · M4 Max turbo layer

**SimSIMD-accelerated kernels, Matryoshka embedding truncation, unified
`TextEmbedder` trait, and Candle-Metal scaffolding.** Every module
feature-gated; pure-Rust default build unchanged. Apple-Silicon-first.

### Shipped

- **`turbo::simsimd_kernels`** — NEON-native `cos_f32` / `dot_i8` / `hamming_b8` / `dot_f32` + batched cos. Feature `simsimd`.
- **`matryoshka`** — `truncate_row` / `truncate_rows` + L2 renormalize. Default-on (pure Rust).
- **`embedder_trait`** — object-safe `TextEmbedder` trait + `EmbedderKind` factory + `SYNAPSE_EMBEDDER` env var. Feature `turbo`.
- **`turbo::mrl_embedder`** — `MrlEmbedder` decorator wraps any backend with MRL truncation. Feature `turbo`.
- **`turbo::candle_metal_embedder`** — Metal BGE-small scaffold (inference path tracked in PR-candle-metal).
- **`turbo::ollama_embedder`** fix: `embed_batch` now uses `/api/embed` single-call path with legacy fallback.
- **`turbo::ndarray_search::search_simsimd`** — feature-gated NEON brute-force NN that skips ndarray Array2 allocation.
- **Cargo features** `simsimd`, `accelerate` (placeholder for Apple Accelerate linking).
- **Docs** `SPEC_V2_M4_MAX_2026-04-24.md`, `M4_MAX_INTEGRATION.md`, `bench_2026-04-24/progression.md`.
- **Bench** `examples/bench_progression.rs` + `benches/kernels_progression.rs` (criterion).

### Measured progression (M4 Max, 100 000 × 384, best-of-three)

| step | kernel | µs/query | QPS | speed-up |
|---|---|---:|---:|---:|
| S0 | scalar cos f32 | 13 210 | 76 | 1.00× |
| S1 | rayon scalar cos f32 | 1 518 | 659 | 8.70× |
| S2 | SimSIMD cos f32 | 763 | 1 310 | 17.30× |
| S3 | SimSIMD dot i8 | 325 | 3 075 | **40.62×** |
| S4 | SimSIMD hamming b8 | 248 | 4 034 | **53.29×** |
| S5 | MRL-128 SimSIMD cos | 396 | 2 522 | 33.32× |

vs Synapse v2.0 Turbo baseline:
- int8 path: 1 284 µs → 325 µs = **3.95×**
- binary path: 661 µs → 248 µs = **2.67×**

### Test + verification

- `cargo build --features "embed,turbo,ollama,simsimd"` ✅
- `cargo build --features "ann-usearch,turbo,simsimd"` ✅
- `cargo clippy --release` ✅ no issues
- `cargo test --lib` ✅ **39 passed** (14 new)

### Out of scope (next PRs)

- Candle-Metal BGE-small real inference (scaffold in place).
- CoreML ANE cross-encoder rerank.
- USearch HNSW live-wiring into `Store::search_vec` (feature already builds).
- Metal 4 topk compute shader.
- `synapse-py` via maturin.

### Rollback

- git tag `backup-pre-m4max-bench-2026-04-24` points at the pre-preview commit.
- tarball: `~/projects/data/synapse-backup-2026-04-24.tgz` (75 MB).

---

## v1.0.0 — 2026-04-20 · World's-breakthrough release

**Synapse 1.0 is the first open single-file format that fuses agent memory,
full-text, vector search, CRDT sync, and signed distribution with sub-20 ms
median latency on every category.** Built in Rust. Shipped in Germany. MIT.

### What shipped together

All the capabilities below live in one `.synx` file, one binary, one crate:

1. **`.synx` v2 container** — Rust-native spec-stable format (`docs/SYNX-FORMAT-V2.md`).
2. **`.brainpack` wrapper** — zstd-wrapped shippable snapshot; Ed25519-signable.
3. **Tantivy BM25** — 5.3 ms build per 10k docs; 23 µs / query.
4. **HNSW + int8 quantization** — 11 ms / 500 queries; 4× smaller vectors.
5. **Temporal knowledge graph** — Supersedes / References / Contradicts /
   Summarises edges with `valid_at` window filters.
6. **Memory scopes** — Global / User / Session / Project (mem0 parity).
7. **Automerge CRDT sync** — union-preserving, deterministic, commutative.
8. **Ed25519 signing** — 25 µs sign + 25 µs verify per manifest hash.
9. **Mmap zero-copy reader** — 0.69 ms cold open on 10 k docs; raw slice in 2 µs.
10. **Python conformance SDK** — stdlib + zstandard + blake3, 200 LOC.

### 50-usecase bench, 900 data points

| category | median latency | median throughput |
|----------|---------------:|------------------:|
| memory  | 0.80 ms | 1 000 ops/s |
| sync    | 2.37 ms | 55 ops/s |
| storage | 8.59 ms | 10 400 docs/s |
| vector  | 9.42 ms | 500 ops/s |
| fts     | 12.84 ms | 200 q/s |

Ten fastest usecases land under 0.34 ms. Full table: `bench/RESULTS-V1.md`.

### Released artefacts

- Version bump 0.3.1 → **1.0.0**.
- `bench/uc_bench.rs` grew from 20 to **50 usecases** across 5 categories.
- `bench/category_summary.py` aggregates per-category medians + best knobs.
- `bench/RESULTS-V1.md` — 50-usecase × CatBoost + world-breakthrough table.
- CHANGELOG now carries the full v0.2.0 → v1.0.0 history.

### Feature parity with incumbents (summary)

| capability | Synapse v1.0 | mem0 | Graphiti | memvid | Meilisearch | LanceDB |
|------------|:-:|:-:|:-:|:-:|:-:|:-:|
| Single-file, portable | ✅ | — | — | ✅ | — | — |
| BM25 + vector + RRF | ✅ | — | partial | — | BM25 only | vector only |
| Temporal KG | ✅ | — | ✅ | — | — | — |
| Memory scopes | ✅ | ✅ | ✅ | — | — | — |
| CRDT multi-writer | ✅ | — | — | — | — | — |
| Signed distribution | ✅ | — | — | — | — | — |
| MCP-native | ✅ | wrapper | wrapper | — | wrapper | wrapper |
| Rust core | ✅ | Python | Python | Rust | Rust | Rust |
| µs-range IPC | ✅ (5.7 µs) | — | — | — | — | — |

## v0.3.1 — 2026-04-20 · Phase-3 rollup + top-20 format comparison

### Added
- **Ed25519 signing** (`synx::sign`) behind `sign` feature. 32-byte keys,
  64-byte signatures over the manifest hash. `generate_key`, `sign_manifest`,
  `verify_manifest`. Roundtrip test included.
- **Automerge CRDT wire** (`sync::automerge_wire`) — encode / merge /
  commutative merges (tests `encode_then_merge_is_union`,
  `merge_is_commutative`).
- **Zero-copy mmap reader** (`synx::mmap::MmapReader`) — opens a 10k-doc
  corpus in ~0.76 ms, exposes raw slices + decoded chunks.
- **HNSW + scalar quantization** (`synx::vec_index`) — `instant-distance`
  backed kNN, `ScalarCodebook` for int8 roundtrip (4× smaller, <0.1 error
  verified in test).
- **Python conformance reader** at `sdk/python/synapse_reader.py`.
  Stdlib + zstandard + blake3; verifies every chunk hash; usable as the
  reference implementation for non-Rust ecosystems.
- **20-usecase Rust bench** (`bench/uc_bench.rs`) + CatBoost-guided
  parameter picker (`bench/catboost_pick.py`). 360 data points, Pareto
  defaults: `zstd_level=3`, `hnsw_ef=16`, usecase importance 95 %.
- **Top-20 format bench** (`bench/top20_formats.py`) — head-to-head across
  SQLite, DuckDB, Parquet, Feather, Arrow IPC, LanceDB, LMDB, DBM, Pickle,
  JSONL+zstd, MessagePack+zstd, CBOR, CSV+gzip, Synapse `.synx`. Results in
  `bench/RESULTS-TOP20.md`.
- **Release plan** (`docs/RELEASE-PLAN.md`) — step-by-step for either a
  re-push of the current history or a clean v1.0.0 squash.

### Changed
- Workspace version → **0.3.1**.
- New feature flag `full = ["fts-tantivy", "mmap", "crdt", "vec-hnsw", "sign"]`
  enables every v0.3 extension in one go.
- `synx::mod.rs` re-exports the v0.3 modules (`MmapReader`, `HnswIndex`,
  `ScalarCodebook`, `FtsIndex`).

### Tests
- default: 20 pass
- `--features fts-tantivy`: 21 pass
- `--features full`: **27 pass** (+ sign roundtrip, automerge commutativity,
  HNSW nearest, quant roundtrip, mmap chunk read, KG chain resolve)

### Research inputs this release
- `agent_top1000_FRESH_20260419.md`: Graphiti / mem0 / Memori / Wax / usearch
- grep.app-style Rust ecosystem sweeps: `instant-distance`, `automerge`,
  `memmap2`, `ed25519-dalek`, `tantivy`
- Top-20 DB-format survey: see `bench/RESULTS-TOP20.md`

## v0.2.4 — 2026-04-20 · Phase-3 build-up (CRDT + mmap + HNSW + SDK)

Same engine features as v0.3.1 but tagged as a v0.2-series rolling milestone.
Kept for historical continuity.

## v0.2.3 — 2026-04-20 · Supply-chain monitoring + dev ergonomics

- `rust-toolchain.toml` pin 1.95.0 + rustfmt/clippy/rust-src
- `.cargo/config.toml` sparse protocol + mold/sccache stanzas
- `deny.toml` license allow-list, bans openssl
- `renovate.json` weekly schedule with grouped core deps
- `.github/workflows/rust-ci.yml` fmt + clippy + test-matrix + audit + deny
- `docs/DEV.md` full top-50 best-practices guide

## v0.2.2 — 2026-04-20 · Tantivy FTS · KG edges · memory scopes

### Added
- **Tantivy FTS wrapper** (`synx::fts::FtsIndex`) behind `fts-tantivy` feature.
  Build 10k-doc index in 131 ms; queries at **0.054 ms/q**, 5× faster than v0.1 FTS5.
- **Temporal knowledge graph** (`synx::kg`) — `Edge`, `EdgeKind`, `EdgeSet`, `Scope`.
  Supersedes / References / Contradicts / Summarises. Valid-at filtering + transitive
  `resolve_current`. Parity with Graphiti without the cluster dependency.
- **Memory scopes** (`synx::kg::Scope`) — Global / User / Session / Project.
  Mem0-style categorisation; carried in RowBatch and KG edge metadata.
- **Release profile tuning** — workspace Cargo.toml sets `lto=thin`,
  `codegen-units=1`, `strip=symbols`, `panic=abort`. ~5–10 % runtime win.
- **Bench script** `bench/bench_v2_features.sh` — end-to-end .synx + Tantivy.
- **Results doc** `bench/RESULTS-V2.md` with head-to-head tables.

### Research inputs
- Scanned `agent_top1000_FRESH_20260419.md`: identified Graphiti (25 k ⭐),
  mem0 (50 k ⭐), cognee (14 k ⭐), Memori (13 k ⭐), Wax (Swift single-file),
  usearch (4 k ⭐). KG + scope + Tantivy close the feature-parity gaps.

### Changed
- `synx::MigrateRow` gained `scope: Option<String>` (backwards-compatible default).
- Tantivy pinned at `0.22` (stable Collector API).

### Tests
- default features: 20 pass
- `--features fts-tantivy`: 21 pass (adds `tantivy_index_and_search`)
- adds 3 KG tests (`scope_tag_roundtrip`, `supersedes_chain_resolved`,
  `valid_at_filters_temporal`, `json_roundtrip`)

## v0.2.1 — 2026-04-20 · `.synx` as the default

(v0.2.0 tag skipped — rebase collision; v0.2.1 is the first clean release of the v0.2 line.)

### Added
- **`.synx` v2 container** — new Rust-native file format.
  Spec: [`docs/SYNX-FORMAT-V2.md`](docs/SYNX-FORMAT-V2.md).
  Reference impl in `crates/synapse-core/src/synx/`.
- **`.brainpack` v2** — distribution wrapper for `.synx`.
  Spec: [`docs/BRAINPACK-V2.md`](docs/BRAINPACK-V2.md).
  API: `synapse_core::BrainPack::{pack, unpack}`.
- **CRDT sync skeleton** — `synapse_core::sync` with `Op` enum and deterministic
  last-writer-wins merge. Wire format is a `CRDTOpsLog` chunk in `.synx`.
- **Signed packs** — Ed25519 slot in the `.synx` footer, enabling paid/trusted
  memory subscriptions.
- **Format RFC + peer-review call** — [`docs/RFC-CALL.md`](docs/RFC-CALL.md).
- **Implementation plan** — [`docs/SYNX-IMPLEMENTATION.md`](docs/SYNX-IMPLEMENTATION.md).
- **Bench scripts** — `bench/bench_synx.sh` (round-trip micro-bench),
  `bench/bench_1m.sh` (1M-doc corpus via existing harness).

### Changed
- README reframed: `.synx` + `.brainpack` are the default path; `.db` is the
  legacy compat engine with a one-way migrate.
- `error::Error` gained a `Format(String)` variant for format-layer errors.
- Workspace depends on `bitflags = "2"`.

### Not removed
- SQLite v1 engine (`db`, `snap`, fastembed, hybrid search, RRF) remains the
  live default while the v2 read-path (Tantivy + HNSW+PQ) is being wired.
  Nothing that worked in v0.1 broke.

### Next
- v0.2.1 — Tantivy-backed FTS chunk kind
- v0.2.2 — HNSW + product-quantized vector chunks
- v0.3.0 — mmap reader, Automerge-backed sync, default-write to `.synx`
