# Synapse Architecture

## 1. Overview

Synapse is a local-first, single-binary hybrid memory store built in Rust. It combines HNSW vector search (via sqlite-vec + usearch), BM25 full-text search (tantivy), a graph layer, CRDT-based sync, and a MCP/daemon interface — all on top of SQLite — delivering sub-millisecond hybrid queries without any external service dependency.

---

## 2. Crate Map

| Crate | Role |
|---|---|
| `synapse-kernel` | ns-tier SIMD distance kernels (S0-S8: f32/f16/int8/1-bit, NEON/AVX-512/SimSIMD) |
| `synapse-core` | Primary store: SQLite + FTS5 + sqlite-vec; CRUD, CRDT ops, snapshot, ed25519 signing |
| `synapse-engine` | ABI layer: RRF fusion, score aggregation, C-callable FFI for engine entry points |
| `synapse-ann` | Pluggable ANN trait + HNSW/usearch backend; swappable without changing core |
| `synapse-fts` | Tantivy-backed FTS index; BM25 scorer, field schema, incremental updates |
| `synapse-graph` | Graph traversal on brain.db foreign-key edges; PageRank, BFS, SQL custom functions |
| `synapse-rerank` | Cross-encoder rerank stage (MiniLM or local ONNX) for SOTA recall pipeline |
| `synapse-rank` | LambdaMART learning-to-rank scaffold (train + inference) |
| `synapse-colbert` | ColBERT-v2 multi-vector late-interaction embedder + store |
| `synapse-splade` | SPLADE-v3 neural sparse inverted index (encoder + block-max index) |
| `synapse-fusion` | MUVERA RRF: dense + ColBERT reciprocal-rank fusion |
| `synapse-extract` | Async fact/entity/type extraction pipeline (NLP → brain.db) |
| `synapse-temporal` | Natural-language temporal phrase parser (chrono + chrono-english) |
| `synapse-space` | Hierarchical Space→Wing→Room→Drawer concept namespace over core |
| `synapse-cluster` | Multi-node AP gossip cluster (TCP transport, CRDT merge) |
| `synapse-raft` | Raft consensus layer for CP replicated store (toggle: RaftMode) |
| `synapse-learn` | Thompson bandit + calibration; adaptive router arm selection |
| `synapse-tune` | Thompson-bandit auto-tuner for SQLite pragmas + batch sizes |
| `synapse-quant` | Vector quantization primitives (scalar, PQ, RaBitQ) |
| `synapse-tier` | Cold-tier blob storage abstraction (local / S3-compat) |
| `synapse-libsql` | Async-WAL SQLite-compat backend (libSQL / Turso remote) |
| `synapse-cms` | Platform-aware query optimizer framework (MySQL/PG/SQLite adapters) |
| `synapse-ops` | Backup, slow-query log, vacuum scheduling |
| `synapse-obs` | OTel traces + Prometheus metrics |
| `synapse-auth` | API-key + RBAC (SHA-256 hash, role table in brain.db) |
| `synapse-server` | MySQL 8 / PG wire-protocol drop-in daemon (opensrv-mysql, pgwire) |
| `synapse-license` | ed25519 license sign/verify + feature-gate enforcement |
| `synapse-metal` | Apple Metal GPU kernel dispatch (embedder acceleration) |
| `synapse-embed-gpu` | Pluggable GPU embedding backend trait (ONNX/Metal/CUDA) |
| `synapse-multimodal` | CLIP-style image+text shared embedding space (feature-gated) |
| `synapse-media` | Multimodal asset-DB: image/video/audio retrieval (ComfyUI/ffmpeg) |
| `synapse-mcp` | MCP stdio JSON-RPC 2.0 bridge → synapsed socket |
| `synapse-cli` | `synx` CLI binary: ping/put/search/hybrid/bench/daemon cmds |
| `synapsed` | Persistent daemon: Unix socket + length-prefixed msgpack; single-writer |
| `synapse-py` | Python bindings (PyO3) |
| `synapse-edge` | Pingora HTTP frontend (opt-in; benchmarked vs axum) |
| `synapse-mysql` | MySQL wire shim for synapsql |
| `synapse-pg` | Postgres wire shim for synapsql |
| `synapse-wal` | Write-ahead log for durable segment writes (scale-100M scaffold) |
| `synapse-seg` | LSM-style segment store for vector partitions (scale-100M scaffold) |
| `synapse-ultra` | High-throughput 4-tier vector search daemon (HNSW via usearch) |

---

## 3. Data Flow

### Put (write path)

```
caller (CLI / MCP / Python)
        │
        ▼
  synapsed  (Unix socket /tmp/synapse.sock)
        │  msgpack decode
        ▼
  synapse-core::put()
    ├─► SQLite INSERT INTO chunks (id, text, meta, vec_blob, blake3, ts)
    ├─► FTS5 index  (tantivy via synapse-fts, or SQLite FTS5 fallback)
    ├─► sqlite-vec INSERT (embedding f16/int8/binary depending on tier)
    └─► graph edge upsert  (if entity links present)
```

### Search (hybrid query path)

```
caller
  │  query + top_k + filters
  ▼
synapsed
  │
  ├─[1] ANN vector search  (synapse-ann / sqlite-vec HNSW)
  │       returns: [(doc_id, score_ann), ...]
  │
  ├─[2] BM25 FTS search    (synapse-fts / tantivy)
  │       returns: [(doc_id, score_bm25), ...]
  │
  ├─[3] RRF fusion         (synapse-engine::rrf_fuse)
  │       k=60, merge both ranked lists → unified score
  │
  ├─[4] Graph boost        (synapse-graph, optional)
  │       PageRank / edge-hop weight added to RRF score
  │
  ├─[5] Rerank             (synapse-rerank, optional cascade)
  │       cross-encoder over top-N candidates → final ranking
  │
  └─► top_k Hits  → caller
```

---

## 4. Kernel Stack

Kernels live in `crates/synapse-kernel/src/kernels/`. Selected at compile time via `cfg(target_arch)` + runtime CPUID.

| Tier | Name | Dtype | Speedup vs scalar |
|---|---|---|---|
| S0 | scalar reference | f32 | 1× |
| S3 | SimSIMD int8 dot | i8 | 46× |
| S4 | SimSIMD hamming / 1-bit | u8 | 71× |
| S5 | MRL-128 subvector | f32 | 35× |
| S8 | SimSIMD f16 dot | f16 | 4× |
| — | NEON int8 dot | i8 | ~20× (ARM fallback) |
| — | NEON f16 dot | f16 | ~8× (ARM fallback) |

Kernels are pure `unsafe` Rust with no stdlib deps; used directly from `synapse-core` and `synapse-ultra`.

---

## 5. Storage Layout

All persistent state lives under `~/.synapse/` (or `SYNAPSE_DATA_DIR`).

```
~/.synapse/
  brain.db            # Primary SQLite DB
    tables:
      chunks          (id, text, meta_json, vec_blob, blake3, created_at, space_id)
      graph_edges     (src_id, dst_id, rel, weight)
      auth_keys       (key_hash, role, expires_at)
      bandit_arms     (arm_id, alpha, beta, last_reward)
      snapshots       (snap_id, ts, blake3_root, enc_blob)
      fts_meta        (last_indexed_ts, doc_count)
  fts/                # Tantivy segment directory
    meta.json
    *.seg             # tantivy segment files
  packs/
    *.synx            # zstd-compressed blake3-addressed chunk blobs
  wal.log             # synapse-wal append log (scale-100M path)
  tier_cold/          # synapse-tier cold blob store
```

Blake3 hashes serve as content-addresses for dedup and integrity checks. Snapshots are age-encrypted (ed25519 recipient).

---

## 6. Concurrency Model

```
┌─────────────────────────────────────────────┐
│               synapsed process              │
│                                             │
│  Unix socket listener  (tokio::net)         │
│        │                                    │
│   N concurrent readers  (tokio tasks)       │
│        │           │                        │
│   read lock      write lock                 │
│   (shared)       (exclusive)                │
│        │           │                        │
│   parking_lot::RwLock<SynapseCore>          │
│        │           │                        │
│   SQLite WAL mode  (multi-reader, 1 writer) │
└─────────────────────────────────────────────┘
```

- Single-writer: all mutations serialize through the RwLock write guard.
- N-reader: queries hold shared read guard; SQLite WAL allows concurrent readers.
- Daemon socket at `/tmp/synapse.sock` (Unix) / `127.0.0.1:9477` (TCP fallback).
- No async I/O inside SQLite: `rusqlite` calls use `tokio::task::spawn_blocking`.
- `synapse-obs` wraps every handler with OTel span + Prometheus histogram.

---

## 7. CRDT Sync

`synapse-core::crdt` uses **yrs** (Yjs Rust port) for Automerge-compatible CRDT ops on document metadata and space hierarchies.

```
local brain.db  ──(yrs Doc ops)──►  op log
                                        │
                              gossip push (synapse-cluster)
                                        │
                              peer brain.db  ◄── merge
```

- **AP mode** (default): `synapse-cluster` TCP gossip; eventual consistency; partition-tolerant.
- **CP mode** (opt-in): `synapse-raft` Raft consensus; toggle `SYNAPSE_RAFT_MODE=1`; requires odd-N quorum.
- Conflict resolution: last-writer-wins on scalar fields; CRDT merge on doc maps.
- Sync scope: metadata + graph edges only. Raw vec blobs replicated separately via `synapse-tier`.

---

## 8. Plugin Points

| Extension | Trait | Where |
|---|---|---|
| Embedder | `EmbedderBackend` (`synapse-embed-gpu`) | swap fastembed → Metal → ONNX |
| ANN backend | `AnnIndex` (`synapse-ann`) | swap sqlite-vec → usearch → custom |
| FTS backend | `FtsIndex` (implicit, via `synapse-fts`) | tantivy or SQLite FTS5 |
| Reranker | `Reranker` (`synapse-rerank`) | MiniLM or any cross-encoder |
| Tier storage | `TierBackend` (`synapse-tier`) | local fs, S3, or custom |
| Query optimizer | `CmsAdapter` (`synapse-cms`) | MySQL / PG / SQLite dialect |

All traits are `Send + Sync`; implementations are injected at daemon startup via config or feature flags.

---

## 9. Performance Notes

Bench baseline: **M4 Max, macOS 24.5, 10k docs, 384-d f16 embeddings** — see [`BENCH_2026-05-10.md`](BENCH_2026-05-10.md).

| Metric | Value |
|---|---|
| Hybrid query latency | **0.023 ms** (vs Qdrant 2.58 ms, LanceDB 2.47 ms) |
| Index size @ 10k docs | **1 290 KB** (vs LanceDB 15 748 KB) |
| FAISS flat crossover | Synapse wins at ≥ 20k docs (FAISS degrades linearly) |
| SimSIMD S4 1-bit peak | 71× scalar throughput |
| SimSIMD S3 int8 | 46× scalar throughput |
| MRL-128 subvector | 35× scalar throughput |
| Daemon socket latency | 8 ms end-to-end hybrid including embed (cached) |
| Throughput (lex-only) | ~300k docs/s PutBatch |

Release profiles: `release` (thin LTO, +5-15%) · `release-fast` (fat LTO, max perf, ~5 min build) · `release-hardened` (stripped, abort, production binary).

---

## 10. Roadmap

Short-term (open gaps → see [`OPTIM_NEXT_2026-05-11.md`](OPTIM_NEXT_2026-05-11.md)):
- Close 7 unmeasured use-cases → SOTA benchmark coverage
- ASHA hyperparameter sweep for RRF k / rerank window
- MLX Metal embedder path (Phase 5)
- Scale-100M: WAL + segment store (`synapse-wal`, `synapse-seg`) graduation from scaffold

Medium-term:
- `synapse-pg` full SQL surface for CRM adapter (synapsql-row)
- Live bench dashboard (Phase 6: [`PHASE-6-LIVE-BENCH-DASHBOARD.md`](PHASE-6-LIVE-BENCH-DASHBOARD.md))
- ColBERT + SPLADE production graduation (currently scaffold)
- Multimodal CLIP embeddings (feature-gated, synapse-multimodal)
