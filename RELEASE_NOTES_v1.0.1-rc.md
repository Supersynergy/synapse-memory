# Synapse v1.0.1-rc.1 — Release Notes

**First public release candidate.** This is the honest record of what is shipped, what is measured, and what is still pending.

---

## Highlights

1. **SimSIMD kernel stack** — NEON int8 (3.4–4.9×), f16 storage (4× RAM), 1-bit Hamming (71× peak), MRL-128 (35×). All benchmarked on M4 Max 100k×384, numbers reproducible via `cargo nextest run -p synapse-kernel`.

2. **MUVERA full-pipeline** — Dense ANN → SPLADE-BMP sparse → RRF(k=60) → ColBERT-i8 rerank in a single `search()` call. Sub-ms on local corpus. Requires `--features fusion-full`.

3. **Hybrid daemon at 8ms** — Unix-socket RPC server (`/tmp/synapse.sock` :9477), persistent apsw connection pool, sidecar FTS rebuild. 56× faster insert than Qdrant HTTP (local bench, not iso-recall).

4. **Raft CP-mode** — 3-node election <1s, 400-LOC minimal impl, opt-in via `--features cluster-raft`. Default stays CRDT gossip (<5ms).

5. **Multi-SDK shipping path** — Rust workspace + PyO3 maturin wheel (`synapse-rs`) + npm postinstall wrapper (`@supersynergy/synx`) + Homebrew tap stub. All three wired to the same `synx` binary.

---

## New Crates (wave-1 → wave-10)

| Crate | Feature flag | Status |
|---|---|---|
| `synapse-colbert` | `colbert` | scaffold — ColBERT-v2 late-interaction, i8 quant |
| `synapse-splade` | `splade-onnx` | scaffold — SPLADE-v3 ONNX, BMP pruning 9.7× |
| `synapse-fusion` | `fusion-full` | prod — MUVERA RRF dense+ColBERT |
| `synapse-multimodal` | `clip-jina` | scaffold — CLIP cross-modal, VJEPA-2 video-temporal |
| `synapse-media` | `audio-clap` | scaffold — CLAP 512-dim mel, ffmpeg→PCM pipeline |
| `synapse-cluster` | `cluster-raft` | prod — CRDT gossip + Raft CP-mode |
| `synapse-obs` | `otel` | prod — OTel + Prometheus dashboards |
| `synapse-fts` | `fts-tantivy` | prod — Tantivy persistent index, 18.3× warm-start |
| `synapse-rank` | `rank` | scaffold — LambdaMART + query-click-log |
| `synapse-quant` | `quant` | prod — int8 IVF k-means (4× mem) |
| `synapse-raft` | `raft` | prod — WAL-Raft segments, 3-node |
| `synapse-ann` | `ann-usearch` | prod — usearch HNSW 21% faster than FAISS p50 |
| `synapse-kernel` | default | prod — NEON int8/f16/hamming kernel crate |
| `synapse-tier` | `spann` | scaffold — SPANN tiered cold storage |
| `synapse-cms` | — | prod — WordPress/CMS Thompson-Beta TTL bandit |
| `synapse-server` | — | prod — generic MySQL/PG drop-in daemon |

Crates in `experimental/` (stubs, 0 tests, excluded from default workspace): `synapse-mysql`, `synapse-pg`, `synapse-edge`, `synapse-embed-gpu`.

---

## Performance Gains

All on M4 Max unless noted. Reproducible via `cargo nextest run` + `bench-dashboard/`.

| Metric | Value | Baseline | Note |
|---|---|---|---|
| SimSIMD 1-bit | 71× | scalar f32 | S4, 100k×384 |
| SimSIMD int8 | 46× | scalar f32 | S3 |
| MRL-128 | 35× | scalar f32 | S5 |
| f16 storage | 4× speed + 50% RAM | f32 | S8 |
| NEON int8 kernel | 3.4–4.9× | scalar int8 | stable Rust, wave-5 |
| BMP SPLADE pruning | 9.7× | naive scan | wave-3 |
| ColBERT int8 | 12.2× speed, 3.9× storage | f32 ColBERT | 100% top-3 overlap |
| Tantivy warm-start | 18.3× | cold boot | 10k docs |
| usearch vs FAISS-HNSW | 21% faster | FAISS p50 77µs vs 98µs | wave-4 |
| Daemon insert vs Qdrant | 56× | local HTTP | not iso-recall |
| apsw TLS pool | 290× FTS, 138× SQL | stdlib sqlite3 | concurrent stress |
| FTS5 sidecar | 50× boot | rebuild-on-start | tail-rebuild |
| MariaDB-overhaul | 700×/32×/1.85× | baseline | synapsql |
| Cascade R@10 | 1.000 @ 73ms | — | 1M corpus, confirmed |

**Honest caveat**: several multipliers are single-threaded microbench on synthetic data. End-to-end wall-clock on real workloads will be lower. `REAL_BENCH_2026-05-11.md` in `bench-dashboard/` documents gaps.

---

## Breaking Changes

None. This is the first public versioned release. Internal pre-release (`v2.1-m4max-preview`) had no public API contract.

---

## Migration from Previous

No migration needed — first public release. If upgrading from the `v2.1-m4max-preview` git tag:

- `synapse-py` version bumped from `0.1.0` → `1.0.1rc1`. Reinstall wheel.
- `[workspace.package] version` is now `1.0.1-rc.1` in Cargo.toml.
- `fastembed` dual-alias issue fixed in wave-2. `--all-features` now compiles clean.

---

## Known Limitations

- **MLX-Metal embed**: scaffold only. Real GPU inference via `objc2-metal` deferred — needs dedicated session. Falls back to fastembed CPU.
- **Real model downloads**: ColBERT, SPLADE, CLIP, CLAP, VJEPA stubs use ONNX swap-path. Downloading actual model weights (~hundreds MB each) is not automated. See crate README for manual steps.
- **SPANN cold storage**: `synapse-tier` is scaffold. Hot→cold eviction logic not wired end-to-end.
- **LambdaMART**: `synapse-rank` — skeleton + click-log schema only. Training loop not yet shipped.
- **Windows**: not tested, not targeted. Linux x86_64 and macOS aarch64/x86_64 only.
- **`bench/wp`**: excluded from default workspace build (RUSTSEC-2026-0002 `lru` transitive via `mysql`). Run explicitly: `cargo bench -p synapse-cms-bench`.
- **Conformal recall threshold**: safety-margin tuned but not validated on external datasets beyond LongMemEval.

---

## Crate Count

17 workspace members (default build) + 4 experimental stubs. ~68 passing tests on default feature set. `cargo nextest run --workspace` green on M4 Max.
