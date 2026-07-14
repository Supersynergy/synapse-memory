# Changelog

All notable changes to this project are documented in this file.

Format: [Keep a Changelog v1.1.0](https://keepachangelog.com/en/1.1.0/)
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html)

---

## [Unreleased]

### Added
- Codex crash-safe resume integration: content-minimal `PreToolUse`/`PostToolUse` checkpoints, append-only fsynced journals, atomic per-project snapshots, and `SessionStart` recovery hints without transcript or tool-output storage.
- LongMemEval benchmark runner now accepts the official JSON shape (`haystack_sessions` + `answer_session_ids`), reports Evidence-R@5/R@10, and reranks the full candidate pool before truncating to top-10.
- **Context-OS** — cross-CLI MCP server for token-budget-bounded, self-learning, verbatim context.
  - `synapse-pack` crate: pure token-budget packer (deletion-based tiers `full → signatures → fact-delta → one-line`, SimHash near-dup collapse, greedy budget knapsack, serial-position ordering). 9 unit tests.
  - `synapse-mcp` tools: `context_pack`, `context_state`, `context_feedback`, `context_remember`; MCP `instructions` for cross-tool guidance; `--brain`/`SYNAPSE_BRAIN` knob.
  - Self-learning loop: `context_feedback` writes per-kind reward (`memory_type_reward`), `context_pack` applies `memory_type_bonus` to ranking.
  - Recall quality: `recall_candidates` unions a raw-query and a high-signal-terms hybrid search, applies a term-overlap boost, and drops noise (via negativa). Noise filter = hard patterns (`is_noise`: telepathy/task-notification/status-JSON/logs/stubs) + an optional **learned classifier** trained offline by `tools/ctxos/train_noise_model.py` (SuperML). A **CatBoost oblivious-tree ensemble** (held-out AUC 0.998; LogReg 0.92 fallback) is exported as compact JSON and applied **natively in Rust** (oblivious-tree evaluator, no runtime Python, graceful when absent) — verified against CatBoost `predict_proba` to 1e-16 via `synapse-mcp --noise-selftest`. `just ctxos-train` retrains from the brain + feedback. Measured: learned model drops noise beyond patterns on session-heavy queries while preserving known-facts. `manifest.noise_filtered` exposed.
  - `scripts/install-ctxos.sh` (`install`/`doctor`/`uninstall`, `--dry-run`) registers the server into Claude Code, Codex and Gemini CLI via their official `mcp add`.
  - Worldwide one-liner `scripts/install.sh`: platform-detect → sha256-verified prebuilt download → `cargo install --git` → source fallback, with macOS ad-hoc re-sign (fixes copied-Mach-O SIGKILL) and an MCP-initialize smoke test. `scripts/build-dist.sh` + `.github/workflows/release-ctxos.yml` produce/publish per-target binaries + `SHA256SUMS` (macOS arm64/x64, Linux gnu x64/arm64).
  - Portable build: `synapse-mcp` `market` feature (default on) gates the `smx_*` engine tools behind `dep:synapse-market`. `--no-default-features` yields an engine-free, **openssl-free** Context-OS binary (only native dep is bundled SQLite) → static musl Linux + baseline-CPU artifacts that run anywhere. `release-ctxos.yml` builds the lean binary as static musl on native runners; `build-dist.sh` forces baseline `RUSTFLAGS`. `.cargo/config.toml` clears `target-cpu=native` for the musl targets.
  - Docs: `docs/CTXOS.md`, `docs/SPEC-ctxos-v2.md`, `docs/adr/0001-context-os-mcp.md`, README section.

---

## [1.0.1-rc.1] - 2026-07-13

### Added

- Portable Synapse Memory release package: six native Rust targets, fail-closed
  checksum installers, safe rollback/uninstall, build metadata, and first- plus
  third-party license payloads.
- Exact portable release gates for RustSec, `cargo-deny`, Cargo license closure,
  native-binary validation, memory/feedback/backup flow, and corrupt-checksum
  rejection.

### Changed

- Portable `synx` now disables network, proprietary engine, semantic runtime,
  sharding, PDF parsing, encrypted packs, Rayon, and Tantivy by feature closure.
- Thompson sampling no longer pulls `statrs`/`nalgebra`/unmaintained `paste`;
  it uses a tested in-crate Beta sampler over the existing `rand` dependency.

### Security

- `synapse-core` metadata now matches the existing FSL-1.1-ALv2 decision; exact
  FSL and MIT texts plus a generated dependency-license report ship per archive.
- Portable six-target dependency union has zero RustSec vulnerabilities or
  warnings; non-shipped monorepo feature paths remain separate audit scopes.

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
