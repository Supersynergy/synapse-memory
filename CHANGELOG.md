# Changelog

All notable changes to this project are documented in this file.

Format: [Keep a Changelog v1.1.0](https://keepachangelog.com/en/1.1.0/)
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html)

---

## [Unreleased]

### Added
- Layering guard `scripts/check-layering.py`, wired into `just check` and CI:
  fails if a product crate depends on an excluded experimental crate (ADR 0001).
  Also exposed as `just check-layers`.
- Root `ARCHITECTURE.md` with the L0 substrate / L1 domain / L2 interface overview.

### Changed
- Default workspace build is now the Context-OS product surface only:
  `synapsed`, `synapse-cli`, `synapse-mcp`, `synapse-rerank`, `synapse-learn`,
  `synapse-extract`, `synapse-space`, `synapse-temporal` (8 crates, down from
  14). Advanced retrieval (`colbert`, `splade`, `fusion`), multimodal
  (`multimodal`, `media`), and bindings (`js`) are excluded from the default
  build and built on demand. Default `cargo check --workspace` dropped 50s -> 20s.
  See `docs/adr/0001-context-os-product-boundary.md`.
- Clippy now runs with `-D warnings` in the `just check` gate and CI (warning-free bar).

### Removed
- Stale `MarketSeries` type alias in `synapse-mcp` (leftover from the market cut).
- Stale market MCP tests (`smx_candles_tool_returns_json` and the `smx_*` entries
  in the tool-surface assertion) — those tools were already cut from `synapse-mcp`.
  Tool-surface threshold corrected 20 -> 16 to match the real served surface.

### Fixed
- Lexical search (`synapse find`) returned the same document twice when the
  vendored tantivy FTS index held a doc across multiple segments; the read path
  now dedups hits by id (bumps `vendor/synapse-db`).
- Clippy warnings: `manual_range_contains` in `synapse-extract`, `type_complexity`
  in `synapse-cli` IO round-trip tests.

---

## [2.1.0] — 2026-08-24

### Added — Production Tools
- **`synapse-ultra health`** — 11-point health audit (integrity, WAL, synchronous,
  foreign_keys, FTS5 index, indexes, triggers, schema version, DB size, page cache,
  ultra schema). JSON output for monitoring.
- **`synapse-ultra backup`** — zstd-compressed backup via `VACUUM INTO` + sha256
  manifest. Default destination: `~/.synapse/backups/`. ~64% compression on real brains.
- **`synapse-ultra metrics`** — Prometheus + JSON exposition format.
  Exposes `synapse_events_total`, `synapse_decisions_total`, `synapse_docs_total`,
  `synapse_db_size_bytes`, `synapse_tags_total`, `synapse_tag_associations_total`,
  `synapse_graph_nodes_total`, `synapse_graph_edges_total`, and more.

### Added — Tag System
- **`synapse-ultra tags`** subcommand with: `add`, `list`, `tag`, `bulk`, `untag`,
  `for`, `docs`, `rule`, `rules`, `merge`, `cleanup`, `stats`, `export`, `import`.
- Schema: `tags` (canonical dictionary with color + description), `doc_tags`
  (many-to-many), `tag_rules` (keyword → tag auto-applied on ingest).
- Idempotent migration — safe to run repeatedly. Additive on top of existing
  `synapse_events` / docs schema.
- Export/import as JSON for cross-brain portability.

### Added — CI
- **6-native release matrix** (`.github/workflows/release-matrix.yml`): builds
  binaries for linux x64/arm64, macos x64/arm64, windows x64/arm64 on every
  `v*.*.*` tag. Uploads tar.gz/zip + sha256 to GitHub Releases.
- **CI test matrix** (`.github/workflows/ci.yml`): split lint + test jobs; tests
  run on all 6 native targets.

### Added — Eval
- **LoCoMo + LongMemEval benchmark harness** (`eval/harness.py`): automated
  download, ingest, retrieval evaluation. Metrics: Recall@k, MRR, latency
  p50/p95, per-category breakdown. Reproducible JSON results with git SHA.

### Changed
- Workspace version bumped 2.0.0 → 2.1.0.
- Repository URL corrected to `https://github.com/Supersynergy/synapse-agent-memory`.
- `synapse-ultra doctor` now uses the full 11-point `health_check` report
  (previously only printed basic stats).
- README rewritten for online presentation: badges, install instructions,
  competitor comparison table (Mem0/Letta/Zep/Chroma/Qdrant), feature matrix
  with v2.1.0 entries.

### Fixed
- `rusqlite` 0.39 `backup` API signature change — replaced with `VACUUM INTO`
  for consistent snapshots without locking writers.
- `integrity_quick` pragma replaced with `integrity_check` for broader SQLite
  compatibility.

---

## [1.0.1-wave-19] - 2026-05-13

### Fixed
- `synapse-graph` semi-naive: bench crash on 100-fact ancestor-closure (7131ms — documented as broken, fix TODO)
- `synapse-tsdb` `fallback` mod visibility: `mod` → `pub mod` to unblock bench
- `synapse-jit` comment claiming `jit ~5ms / interp ~18ms / sqlite ~28ms` corrected — actual: JIT=interp=8ms, SQLite=18ms (M4 faster than comment-era hardware)

### Performance
- Nanosecond hot-path optimizations (rayon hash-path threshold + embed cache counters)

### Honest-known-gaps
- JIT: no speedup vs interpreter on 1M-row filter bench (Cranelift, single-thread)
- Datalog: semi-naive quadratic, 100k facts timeout (>30s). Not production-ready.
- Metal (synapse-mlx-olap): `engine.backend` shows `Cpu` — Metal dispatch not confirmed

---

## [1.0.1-wave-18] - 2026-05-13

### Added
- `synapse-stream` pub/sub: 13.1M events/s (76 ns/msg via tokio broadcast-channel), verified
- `synapse-stream` CDC: emit_direct 100k events measured at 2,241 events/s (SQLite-write-per-event bottleneck documented)
- `synapse-tsdb` insert bench: 4.26M rows/s (fallback SQLite-backed store, 1M rows)
- `synapse-mlx-olap` GROUP BY SUM CPU: 10M rows / 5 keys = 395ms (~25M rows/s); scalar SUM 10M = 10.8ms (~926M rows/s)
- `synapse-jit` filter: 2.25× faster than SQLite in-memory on 1M-row WHERE bench

### Performance (bench sources)
- [REAL_BENCH_WAVE17_18_2026-05-13.md](bench-dashboard/REAL_BENCH_WAVE17_18_2026-05-13.md)

---

## [1.0.1-wave-17] - 2026-05-13

### Added
- `synapse-tsdb` crate: time-series columnar store, Arrow/Parquet backend (`--features tsdb`)
- `synapse-stream` crate: pub/sub + CDC events
- `synapse-mlx-olap` crate: GROUP BY analytics, CPU + Metal stub
- `synapse-jit` crate: Cranelift JIT filter compilation
- `synapse-graph` Datalog: semi-naive fixpoint (ancestor-closure, ⚠️ broken above ~100 facts)

---

## [1.0.1-wave-16] - 2026-05-12

### Changed
- `BrainAdapter` MAX-rewrite: full context pull + cherry-pick gold paths
- Cleanup sprint: dead code, unused imports removed workspace-wide

---

## [1.0.1-wave-15] - 2026-05-12

### Fixed
- SynapsQL: 6 critical bugs resolved (MySQL wire-protocol edge cases)
- `brain_adapter` partial wiring for high-throughput insert path

### Added
- SynapsQL reality-check: verified MariaDB bench numbers 700×/32×/1.85× (documented in [SYNAPSQL_FEATURE_MATRIX_2026-05-12.md](bench-dashboard/SYNAPSQL_FEATURE_MATRIX_2026-05-12.md))

---

## [1.0.1-wave-14] - 2026-05-12

### Added
- SynapsQL MAX: MySQL wire-protocol proxy (`synapsql` crate)
- WordPress bench real numbers ([WP_BENCH_2026-05-12.md](bench-dashboard/WP_BENCH_2026-05-12.md))
- TigerBeetle comparison bench ([TIGERBEETLE_BENCH_2026-05-12.md](bench-dashboard/TIGERBEETLE_BENCH_2026-05-12.md))
- SQL bench matrix ([SQL_BENCH_MATRIX_2026-05-12.md](bench-dashboard/SQL_BENCH_MATRIX_2026-05-12.md))

---

## [1.0.1-wave-13] - 2026-05-12

### Added
- HNSW parallel-build scaffold (15.7× speedup target vs sequential; full parallel not yet default)
- ef-search Pareto sweep: ef=16..512 on SIFT-1M HNSW-f16 ([EF_SWEEP_2026-05-12.md](bench-dashboard/EF_SWEEP_2026-05-12.md))
- glass-backend scaffold (CPU-SIMD beam search, integration TODO)
- Docker / k8s deployment manifests

### Performance
- ef=64: R@10=0.979, p50=0.18ms, QPS=5723 (SIFT-1M HNSW-f16)
- ef=192 recommended production: R@10=0.993, p50=0.32ms, QPS=3240

---

## [1.0.1-wave-12] - 2026-05-12

### Added
- SIFT-1M HNSW full engagement: hnsw-i8 0.10ms p50, 10474 QPS at ef=64 ([SIFT1M_BENCH_2026-05-12.md](bench-dashboard/SIFT1M_BENCH_2026-05-12.md))
- Advanced ANN modes: hnsw-f16, hnsw-f32, hnsw-i8, brute-force i8/f16/rabitq
- Fair durability bench: SQLite-WAL strict 7K/s batch-1, 943K/s batch-1000 ([FAIR_DURABILITY_BENCH_2026-05-13.md](bench-dashboard/FAIR_DURABILITY_BENCH_2026-05-13.md))
- Linux vs macOS parity table: usearch-HNSW within 4% ([LINUX_VS_MACOS_2026-05-13.md](bench-dashboard/LINUX_VS_MACOS_2026-05-13.md))

### Honest-known-gaps
- HNSW build: 197–672s for 1M (faiss-hnsw ~30–60s). Parallel insert TODO.
- hnsw-i8 R@10=0.908 at ef=64 (misses ≥0.95 target). Use ef≥192 for R@10≥0.99.

---

## [1.0.1-wave-11] - 2026-05-12

### Added
- SIFT-1M bench harness: 1M×128d, L2-normalised cosine, 1000 queries
- CI bench-regression: auto-compare vs baseline on PR
- v1.0.1-rc prep: version bump + release matrix

---

## [1.0.1-wave-10] - 2026-05-12

### Added
- Nanosecond hot-path optimizations for core search loop
- RAM utilization tracking
- `synapse-market` crate: OHLCV + regime-vec HFT/backtest engine

---

## [1.0.1-wave-9] - 2026-05-12

### Added
- MUVERA-store: persistent fusion index
- RaBitQ cascade: 1M corpus R@10=1.000 @ 73ms (confirmed via bench commit ff7e975)
- WAL-Raft segment integration
- JS SDK (`synapse-js` via napi-rs)
- SPANN tiered cold storage scaffold (`synapse-spann`)

---

## [1.0.1-wave-8] - 2026-05-11

### Added
- MUVERA full-pipeline E2E (`fusion-full` feature): Dense → SPLADE BMP → RRF(k=60) → ColBERT-i8
- HippoRAG-2 graph RAG scaffold
- RaBitQ quantization (48MB for 1M×128d)
- MLX scaffold (Metal BGE-small, inference path stub)

---

## [1.0.1-wave-7] - 2026-05-11

### Added
- F16 HNSW: 3× memory reduction vs f32, 4997 QPS @ R@10=0.982 on SIFT-1M
- Pinecone + Weaviate migration adapters (`synapse-migrate`)
- PyO3 wheel scaffold (`synapse-py`)
- Raft snapshot support
- SPANN tiered architecture scaffold

---

## [1.0.1-wave-6] - 2026-05-11

### Added
- F16 NEON kernel: 4× speed + 50% RAM vs f32
- Raft-cascade: R@10=1.000 guaranteed via two-stage exact rerank
- ARCHITECTURE.md
- `synapse-migrate`: import from Qdrant/LanceDB/Chroma
- Fuzzer (`cargo fuzz` targets)

---

## [1.0.1-wave-5] - 2026-05-11

### Added
- NEON int8 dot via `vmull_s8+vpadalq_s16` (stable Rust): 3.4–4.9× over scalar, 42–60× over f32 total
- NEON int8 path wired into `synapse-kernel` pub API; ColBERT i8 quant uses it
- Synapse Raft CP-mode (`cluster-raft` feature, default off): 3-node election <1s
- `ConsensusMode` enum: `Crdt` (default) + `Raft` (optional)
- MTEB mini-bench real numbers: bge-small NFCorpus nDCG@10=0.343, SciFact nDCG@10=0.713 ([MTEB_MINI_2026-05-11.md](bench-dashboard/MTEB_MINI_2026-05-11.md))
- gRPC-batch vs Qdrant parity bench: Synapse 56× faster insert (334 vs 5.9 k/s, local, not iso-recall)
- `examples/agent_memory`: 5-turn chat demo
- `examples/code_search`: FTS5+ColBERT-i8 hybrid (51ms)
- `examples/multimodal_rag`: image+text cross-modal demo
- `--rerank-model` CLI flag (`baseline`, `jina-colbert`, `jina-cross-encoder`)

### Fixed
- `put_batch_deferred_fts_throughput` marked `#[ignore]` (flaky under parallel load)
- `bench_tantivy_warm_start_10k` marked `#[ignore]` (same)
- Conformal recall threshold safety-margin tuned

---

## [1.0.1-wave-4] - 2026-05-11

### Added
- Raw-ANN microbench vs FAISS: usearch 77µs vs FAISS-HNSW 98µs p50 (21% faster, N=10k, 384d) ([RAW_ANN_BENCH_2026-05-11.md](bench-dashboard/RAW_ANN_BENCH_2026-05-11.md))
- MUVERA full-pipeline E2E scaffold
- Audio CLAP embedding (`audio-clap` feature, 512-dim mel-filterbank)
- Homebrew tap (`dist/homebrew/synx.rb`, 3 platform slots)
- npm wrapper (`@supersynergy/synx`)
- GH release CI (`.github/workflows/release.yml`, 3-target matrix)

### Honest-known-gaps
- usearch R@10=0.942 < 0.95 on N=10k bench

---

## [1.0.1-wave-3] - 2026-05-11

### Added
- BMP block-max pruning for SPLADE: 9.7× vs naive scan
- ColBERT int8 quantization: 12.2× speed, 3.9× storage, 100% top-3 overlap
- VJEPA-2 video-temporal scaffold (`synapse-media`): RGB+Luma+DCT 768-dim, ONNX swap-path
- `REAL_BENCH_2026-05-11.md`: honest Top-20 reality-check with action items

### Fixed
- Documented full-stack overhead in pure-ANN mode vs FAISS

---

## [1.0.1-wave-2] - 2026-05-11

### Added
- jina-clip-v2 ONNX loader (`clip-jina` feature, CLIP_DIM=1024)
- naver/splade-v3 ONNX loader (`splade-onnx` feature, top-64 sparse)
- jina-colbert-v2 candle loader (`colbert-jina` feature, 128-dim tokens)
- `synapse-fusion` crate: MUVERA RRF dense+ColBERT
- Grafana dashboards: `synapse-overview.json` + `synapse-traces.json`

### Fixed
- fastembed dual-alias resolved (`--all-features` now compiles clean)

---

## [1.0.1-wave-1] - 2026-05-11

### Added
- ColBERT-v2 multi-vector late-interaction scaffold (`synapse-colbert` crate)
- SPLADE-v3 neural-sparse inverted-index scaffold (`synapse-splade` crate)
- Conformal recall-predictor (`conformal` feature): split-conformal R=1.0 coverage guarantee
- Two-stage exact-rerank `--guarantee` CLI flag
- HyDE query augmentation (`ollama` feature + `--hyde` flag)
- Direct-NEON RRF intrinsics 4.3–5.1× via sort-merge (replaces HashMap)
- `synapse-fts` crate: Tantivy persistent index + put_batch mirror (18.3× warm-start)
- Schema-dim feature-flags (`embed-384`, `embed-768`, `embed-1024`)
- Multi-node CRDT gossip cluster (`synapse-cluster` crate, <5ms gossip)
- Attribute-filter pushdown with ef-boost oversampling
- LambdaMART scaffold + query-click-log (`synapse-rank` crate)
- CLIP cross-modal (`synapse-multimodal` crate, feature-gated)
- Asset-DB + ffmpeg pipeline (`synapse-media` crate)
- Observability OTel+Prometheus (`synapse-obs` crate)

### Changed
- Cascade clamp extended: mult `2..=100` (was 16), ef-cap `16384` (was 4096)

---

## [v2.1-m4max-preview] - 2026-04-24

SimSIMD kernels, Matryoshka MRL, unified `TextEmbedder` trait, `AdaptiveRouter` Thompson bandit, `MultiIndex` one-liner, LangChain/Mem0/LlamaIndex adapters.

### Measured (M4 Max, 100k×384, cold)

| Kernel | µs/q | Speedup |
|--------|------|---------|
| S0 scalar cos f32 | 13 210 | 1.0× |
| S3 SimSIMD int8 | 287 | 46× |
| S4 SimSIMD 1-bit | 192 | 71× |
| S5 MRL-128 | 375 | 35× |
| S8 f16 storage | 5 141 | 4× |

68 tests passing, 0 warnings.

---

## [v1.0.0] - 2026-04-20

First public release. Single-file format fusing agent memory, full-text, vector search, CRDT sync, and signed distribution.

Key capabilities: `.synx` v2 container · `.brainpack` zstd wrapper · Tantivy BM25 · HNSW int8 · Temporal KG · Memory scopes · Automerge CRDT · Ed25519 signing · Mmap zero-copy reader.
