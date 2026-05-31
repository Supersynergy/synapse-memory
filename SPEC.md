# Synapse — Canonical Spec v1.0

**Date**: 2026-05-25  
**Branch**: `main` / Context OS release slice  
**Replaces**: `synapsestore/docs/legacy-specs/SPEC_V2_M4_MAX_2026-04-24.md` (kernel/phase tracker) and `synapsestore/docs/legacy-specs/SPEC_AGENTMD.md` (AgentMD ingestion spec, out-of-scope)

---

## Mission

Local-first Context OS for AI agents: every prompt should start with the best
available context, bounded to budget, cited, freshness-aware, and improved by
feedback. The vector/FTS/graph/database work is substrate, not the default
first-run product promise.

## Current Release Scope

The current clean-user release lives in `release/context-os/`.

Verified release commands:

```bash
release/context-os/verify.sh
SYNAPSE_VERIFY_INSTALL=1 SYNAPSE_VERIFY_BUILD_PROFILE=dev release/context-os/verify.sh
SYNAPSE_PACKAGE_DRY_RUN=1 release/context-os/package.sh
SYNAPSE_SERVICE_OS=Darwin SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install
SYNAPSE_SERVICE_OS=Linux SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install
```

Release invariants:

- No maintainer `brain.db`, embedding cache, Claude/Codex session logs, or
  private memories are shipped.
- Source package is buildable from an extracted tarball.
- `synx prime`, `context`, `remember`, `feedback`, `fresh-context`, and
  `doctor --fix` are the default user workflow.
- Graph/OLAP/TSDB/SQL-wire/multimodal surfaces are not default release claims.

The historical hard targets below remain engine targets unless explicitly
covered by `release/context-os/VERIFICATION.md`.

---

## Hard Targets

All numbers verified from bench runs in this repo.

| Metric | Target | Source |
|--------|--------|--------|
| Insert single-thread | ≥ 50 000 ops/s | Python adapter: 4 760 ops/s; Rust direct: **TBD** — run `cargo bench -p synapse-core -- insert` to establish baseline |
| Insert 4-thread | ≥ 250 000 ops/s | **TBD** — rayon path exists; run `cargo bench -p synapse-core -- insert_parallel` to establish baseline |
| FTS query p50 | ≤ 0.05 ms | criterion: 51 µs/query ✓ (already hit) |
| Hybrid query p50 | ≤ 0.10 ms | target; FTS at 51 µs + vec overhead |
| Single embed | ≤ 2.5 ms | Ollama/FastEmbed path; BGE-small-en-v1.5 |
| Batch embed (MLX) | ≤ 0.25 ms/doc | Metal path via synapse-metal (wired stub) |
| Storage overhead | ≤ 2× sqlite-vec | .synx at 1 285 KB vs SQLite 1 432 KB — ✓ (0.9×) |
| Concurrency saturation | 4–8 threads | rayon global pool; M4 Max 16P cores |
| LongMemEval R@5 | ≥ 0.85 | current: 0.30 (per-message, no reranker); needs cross-encoder rerank (ROADMAP P1) |

---

## Architecture — 15 Active Crates

| Crate | Role |
|-------|------|
| `synapse-core` | Store, chunk CRUD, FTS5 (Tantivy bridge), vector index (sqlite-vec), KG triples, zstd/blake3 I/O. **API frozen at v1.** |
| `synapse-engine` | ABI bridge + RRF fusion: wires FTS+vec results into ranked output |
| `synapse-space` | Agent-memory layer: Space→Wing→Room→Drawer hierarchy, sweep/compact/evolve ops |
| `synapsed` | Unix-socket RPC daemon; multiplexes core across callers without re-opening DB |
| `synapse-cli` (`synx`) | CLI: `synx put`, `synx hybrid`, `synx find`, `synx stats`, `synx merge`, `synx sign`, `synx verify` |
| `synapse-mcp` | MCP server: `synapse_search`, `synapse_put`, `synapse_find`, `synapse_stats`, `synapse_merge`, `synapse_verify` |
| `synapse-learn` | Bandit router (Thompson sampling), per-query calibration, EWMA feedback |
| `synapse-rerank` | Cross-encoder rerank; `IdentityReranker` default, `OnnxCrossEncoder` with `--features onnx` |
| `synapse-extract` | Text extraction and chunking: per-message, fixed-window, semantic boundary |
| `synapse-temporal` | NL date phrase parser via chrono-english; bitemporal filter |
| `synapse-metal` | Metal/ANE compute kernels: SimSIMD cos_f32/dot_i8/hamming_b8, MRL truncation |
| `synapse-ann` | Scale-100M ANN scaffold: HNSW live-wire, PQ quantisation (stub, P0 TODO) |
| `synapse-quant` | Quantisation helpers: f32→i8, f32→f16, f32→binary; matryoshka MRL. **Experimental — unvalidated on production workloads.** |
| `synapse-license` | License key validation (embedded binary check) |
| `synapse-py` | PyO3 Python wheel: `synapse.Brain`, `synapse.MultiIndex`, `synapse.AdaptiveRouter`, integrations (LangChain, Mem0, LlamaIndex) |

> **Note**: `synapse-wal` (crash-safe ingest WAL, PR-E1) and `synapse-seg` (LSM segment store, PR-B1) are scaffolding stubs in `synapsestore/crates/` — not part of active workspace build.

---

## Out of Scope

- **Distributed / cluster mode** — single-node embedded only. See ADR-001 (`docs/adr/ADR-001-no-distributed.md` TBD).
- **Python runtime in core** — synapse-py is a thin PyO3 wrapper; no Python in the Rust hot path.
- **Mojo backend** — not planned; Metal kernel covers same use case.
- **Cloud sync** — no cloud dependency; external sync is user responsibility.
- **Full-text MySQL wire protocol** — moved to `synapsestore/crates/synapse-mysql`.

---

## Active Project Surface

| Surface | Entry | Notes |
|---------|-------|-------|
| Memory layer | `crates/synapse-space` | Space::add, search, search_reranked, sweep, evolve |
| Daemon | `crates/synapsed` | `/tmp/synapse.sock` by default |
| CLI | `crates/synapse-cli` | `synx` binary |
| MCP | `crates/synapse-mcp` | Port 3000 default |
| Python wheel | `crates/synapse-py` | `maturin develop` |

---

## Public API Stability

- `synapse-core` types (`Store`, `Chunk`, `ChunkId`, `SearchResult`, `KgTriple`): **frozen at v1**. Breaking changes require a major version bump and migration guide.
- All other crates: SemVer, minor versions may have additive changes, patch versions are backward-compatible fixes.
- `synapse-py` bindings: follow core stability; additional helpers may be added in minor releases.

---

## Config Defaults (CatBoost-tuned, 2026-04-20)

From `RESULTS-V2-FULL.md` — 360-point sweep, feature importance: usecase 95.1%, corpus size 4.5%, zstd 0.22%, hnsw_ef 0.21%.

```
zstd_level = 3      # fastest compression; matches zstd=19 within margin of error on text
hnsw_ef    = 16     # sub-10 ms at ≤100k vectors; increase to 64 at >1M
mmap       = true   # 0.76 ms open; near-free, always enable
journal_mode = WAL  # standard production SQLite setting
synchronous  = NORMAL
```

Key insight: **engine routing dominates latency (95%); knobs contribute <1%**. Pick the right path (Tantivy for lex, HNSW for kNN, mmap for open) before tuning parameters.

---

## LongMemEval Roadmap (R@5: 0.30 → 0.85+)

| Step | Implementation | Est. R@5 gain |
|------|---------------|---------------|
| P0 | Per-message chunking (✓ done) | 0.00 → 0.30 |
| P1 | Cross-encoder reranker (`synapse-rerank` ONNX) | +0.30–0.40 |
| P2 | BM25 pre-filter + HyDE query expansion | +0.10–0.15 |
| P3 | `space_sweep` + entity KG + `drawer_evolve` | +0.05–0.10 |

---

## Reproduce

```bash
# Core check
cargo check -p synapse-core

# Full workspace
cargo build --workspace --release

# MemPalace shootout (Python, needs venv)
cd bench/mempalace-shootout
python run.py

# LongMemEval (Rust criterion)
cargo bench -p bench-space-vs-chroma

# Auto-tune sweep
cd bench/auto-tune
python harness.py        # 30-config random search, ~5 min
python tune.py           # CatBoost picks winner, writes BEST_CONFIG.json
```
