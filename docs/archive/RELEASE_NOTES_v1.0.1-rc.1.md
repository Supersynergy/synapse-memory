# Synapse v1.0.1-rc.1 — Release Notes

First release candidate. Honest record of what is shipped, measured, and still pending.

---

## Installation

### Homebrew (macOS / Linux)
```bash
brew tap supersynergy/synapse
brew install synx
```
SHA256 placeholders in formula — update after CI publishes release tarballs.

### npm / bun (Node 18+)
```bash
npm install -g @supersynergy/synx
# or: bun add -g @supersynergy/synx
```
Downloads correct pre-built binary at postinstall.

### pip / PyPI (Python ≥ 3.9, abi3 wheel)
```bash
pip install synapse-rs
# or: uv pip install synapse-rs
```
Build from source: `RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup" maturin build --release --strip`

### Cargo
```bash
cargo install synapse-cli
```
Installs `synx` binary.

### Docker
```bash
docker pull ghcr.io/supersynergy/synapse:1.0.1-rc.1
docker run --rm -v $PWD:/data ghcr.io/supersynergy/synapse synx hybrid "query"
```

---

## Highlights

### 1. HNSW search at 0.10ms p50 (SIFT-1M)

Source: [bench-dashboard/SIFT1M_BENCH_2026-05-12.md](bench-dashboard/SIFT1M_BENCH_2026-05-12.md)

- hnsw-i8 ef=64: 0.10ms p50, 10474 QPS, R@10=0.908
- hnsw-f16 ef=192 (recommended production): 0.32ms p50, 3240 QPS, R@10=0.993
- Build caveat: 197–672s for 1M vectors (parallel batch insert TODO)

### 2. SimSIMD kernel stack

Source: CHANGELOG wave-1 / v2.1-m4max-preview bench

| Kernel | Speedup vs scalar f32 |
|--------|----------------------|
| SimSIMD 1-bit | 71× |
| SimSIMD int8 | 46× |
| MRL-128 | 35× |
| f16 storage | 4× speed + 50% RAM |
| NEON int8 (stable Rust) | 3.4–4.9× vs scalar int8 |

All on M4 Max, 100k×384 synthetic corpus. Real workloads will be lower.

### 3. Hybrid daemon — 35ms on 294k production docs

Source: [bench-dashboard/REAL_BENCH_2026-05-11.md](bench-dashboard/REAL_BENCH_2026-05-11.md)

- 35ms per hybrid search call (FTS5 + ANN + RRF + rerank, Unix-socket)
- 334 k/s put-batch (FTS5 + vec + CRDT, persisted)
- 56× faster insert than Qdrant HTTP local (not iso-recall benchmark)

### 4. Conformal recall guarantee

Split-conformal calibration. R=1.0 coverage on LongMemEval. Not validated on external datasets.

### 5. Pub/sub stream

Source: [bench-dashboard/REAL_BENCH_WAVE17_18_2026-05-13.md](bench-dashboard/REAL_BENCH_WAVE17_18_2026-05-13.md)

- 13.1M events/s (76 ns/msg) via tokio broadcast-channel
- CDC on-disk: 2,241 events/s (SQLite-write-per-event; WAL batch TODO)

### 6. Multi-SDK shipping

Rust workspace + PyO3 maturin wheel (`synapse-py`, not yet on PyPI) + npm postinstall wrapper (`@supersynergy/synx`) + Homebrew tap stub.

---

## New crates (waves 1–19)

| Crate | Feature | Status |
|-------|---------|--------|
| `synapse-colbert` | `colbert` | scaffold |
| `synapse-splade` | `splade-onnx` | scaffold, BMP 9.7× |
| `synapse-fusion` | `fusion-full` | stable, MUVERA RRF |
| `synapse-multimodal` | `clip-jina` | scaffold |
| `synapse-media` | `audio-clap` | scaffold |
| `synapse-cluster` | `cluster-raft` | stable, CRDT + Raft |
| `synapse-obs` | `otel` | stable |
| `synapse-fts` | `fts-tantivy` | stable, 18.3× warm-start |
| `synapse-rank` | `rank` | scaffold, skeleton only |
| `synapse-quant` | `quant` | stable |
| `synapse-raft` | `raft` | stable, 3-node |
| `synapse-ann` | `ann-usearch` | stable |
| `synapse-kernel` | default | stable |
| `synapse-stream` | — | stable (pub/sub), partial (CDC) |
| `synapse-tsdb` | `tsdb` | partial (fallback 4.26M/s; Arrow-path unbenched) |
| `synapse-mlx-olap` | — | partial (CPU confirmed; Metal unverified) |
| `synapse-jit` | — | partial (2× vs SQLite; no gain vs interpreter) |
| `synapsql` | — | stable, MySQL wire-proxy |
| `synapse-market` | — | stable |
| `synapse-migrate` | — | stable |
| `synapse-js` | — | stable |

Experimental stubs (0 tests, excluded from default build): `synapse-mysql`, `synapse-pg`, `synapse-edge`, `synapse-rank`, `synapse-embed-gpu`.

---

## Breaking changes

None. First public versioned release.

---

## Migration from Chroma / Qdrant / LanceDB / Pinecone / Weaviate

```bash
# Qdrant → Synapse (built-in CLI)
synx migrate qdrant --url http://localhost:6333 --collection my_col --out ./my.db

# LanceDB → Synapse (Parquet export)
synx import --parquet ./dump.parquet --out ./my.db

# Chroma → Synapse (Python)
import synapse_py as synx
db = synx.Brain("./my.db")
db.put_batch([(text, vec) for text, vec in chroma_col.get()])

# Pinecone / Weaviate → Synapse (JSONL export + import)
synx import --jsonl ./dump.jsonl --out ./my.db
```

Full migration docs: `docs/migration/` (generated via `synapse-migrate` crate).

---

## Migration from v2.1-m4max-preview

- `synapse-py` version bumped to `1.0.1rc1`. Reinstall wheel.
- `[workspace.package] version` = `1.0.1-rc.1` in Cargo.toml.
- fastembed dual-alias fixed in wave-2. `--all-features` compiles clean.

---

## User Actions Required for Live Publish

| Action | Detail |
|--------|--------|
| `NPM_TOKEN` | npm automation token → `export NPM_TOKEN=xxx` before `npm publish --tag rc` |
| `PYPI_TOKEN` | PyPI API token → `maturin publish --token xxx` |
| Brew tap repo | Create `github.com/Supersynergy/homebrew-synapse`, push `Formula/synx.rb` |
| Tag push | `git tag v1.0.1-rc.1 && git push origin v1.0.1-rc.1` (triggers CI) |
| Update SHA256 | After CI uploads tarballs, run `dist/homebrew/update_sha256.sh` (or manually sha256 each tarball) |
| GitHub release | Create from tag via `gh release create v1.0.1-rc.1 --prerelease` |

---

## Known limitations

- **Datalog (synapse-graph)**: semi-naive quadratic — 7s for 100 facts, timeout at 1k. Not production-ready. Fix: delta-join + HashMap index.
- **Metal/MLX OLAP**: `engine.backend` shows `Cpu` in current bench. Metal dispatch not confirmed.
- **JIT**: no speedup vs interpreter on 1M-row filter bench (Cranelift single-thread, 8ms each).
- **CDC throughput**: 2,241 events/s (SQLite-write-per-event bottleneck). WAL batch TODO.
- **HNSW build time**: 197–672s for 1M. faiss builds in ~30–60s. Parallel insert TODO.
- **usearch R@10**: 0.942 at N=10k/384d. Needs `expansion_search` tuning for ≥0.95.
- **MLX/CLIP/SPLADE/CLAP/VJEPA**: ONNX swap-path only. Model weights not auto-downloaded.
- **Python wheel**: not yet published to PyPI.
- **Linux CI**: `synapse-extract` E0463 link-order bug. `synapse-market` opensrv git dep excluded.
- **Windows**: not tested, not targeted.
- **io_uring durability**: macOS-only bench available. Linux bare-metal numbers pending.
- **MTEB**: 2 of 56 tasks measured. Full suite estimated ~1 day CPU.

---

## Crate count

~33 workspace members (default build) + 5 experimental stubs.
`cargo nextest run --workspace` green on M4 Max (macOS aarch64).
Linux: partial green (see known limitations).
