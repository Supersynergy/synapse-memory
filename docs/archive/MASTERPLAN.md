# Synapse — Single-File Memory, At SQLite Speed

> MV2-portability. FTS5-speed. Daemon-mode. Rust core.
> One file. No spawn. No Python. No lock-in.

**Status:** MASTERPLAN (2026-04-19) — pre-code, architecture-locked
**Author:** Maxim Supersynergy
**License:** MIT

---

## 1. Problem

memvid's `.mv2` format is a clever single-file memory store for AI agents (Rust, Tantivy+HNSW, embedded WAL). But benchmarks on an M4 Max (2026-04-19) show it lose decisively to plain SQLite+FTS5 on the same workload:

| Op (200 docs, M4 Max) | MV2 CLI | SQLite FTS5 | Δ |
|---|---|---|---|
| Insert 200 | 29.55s | 0.37s | **80× slower** |
| Lex search | 12450ms/q | 28ms/q | **450× slower** |
| File size | 1.12 MB | 94 KB | 12× bigger |
| Cold-start | CLI spawn 200ms | in-proc 0ms | — |

Root-causes:
1. **CLI spawn per call.** Every `memvid find` reloads the full binary + Tantivy index.
2. **Synchronous per-doc embedding.** No batching across ANE/GPU.
3. **Bundle-format overhead.** 4KB header + 1-64MB WAL + Tantivy segment metadata even on tiny datasets.

MV2 wins on one axis only: **portability** (one file, no sidecars, git-commitable). That single-file property is worth preserving — everything else is fixable.

## 2. Goal

Build a memory layer that **keeps MV2's portability** but **matches SQLite+FTS5 speed** (and exceeds it for embeddings). Targets on M4 Max, 1k docs:

| Op | Target | vs MV2 | vs SQLite bare |
|---|---|---|---|
| Insert 1k docs (with embed) | <500ms | 60× faster | on-par |
| Lex query | <2ms p95 | 6000× faster | on-par (FTS5) |
| Vec query | <6ms p95 | 15× faster | on-par (sqlite-vec) |
| Hybrid query | <8ms p95 | n/a | new |
| File size (1k docs) | ~500KB | 10× smaller | 5× bigger (has vectors) |
| Daemon cold-start | <50ms | 4000× faster | n/a |
| Socket-call RTT | <0.2ms | 1000× faster | n/a |

Non-goals: replace SuperKnow v2, replace production OLAP, replace Qdrant at >10M vectors.

## 3. Architecture

```
┌──────────────────────────────────────────────────────┐
│  clients: node-sdk, python-sdk, cli (all thin)       │
│           wire: msgpack over unix socket             │
└──────────────────────────────────────────────────────┘
                         │
┌──────────────────────────────────────────────────────┐
│  synapsed  (Rust, persistent daemon, :socket)        │
│    ├── request router (tokio)                        │
│    ├── embed queue (batch-256, ANE via ONNX)         │
│    ├── query planner (lex / vec / hybrid / time)     │
│    └── snapshot engine (.brainpack export/import)    │
└──────────────────────────────────────────────────────┘
                         │
┌──────────────────────────────────────────────────────┐
│  storage (SQLite WAL, single file)                   │
│    ├── docs (id, title, text, meta JSONB, ts)        │
│    ├── docs_fts (FTS5 virtual table, porter stem)    │
│    ├── docs_vec (sqlite-vec, HNSW, 384-dim BGE-small)│
│    ├── blobs (zstd-compressed raw, BLAKE3-deduped)   │
│    └── meta (schema version, embed model, capacity)  │
└──────────────────────────────────────────────────────┘
                         │
              brain.db  (single file — that's it)
                  + brain.db-wal (transient, checkpointed)
```

On export: `synapse snap foo.brainpack` → checkpoint WAL → zstd(brain.db) → one portable file.
On import: reverse. No sidecars ever leave the machine.

## 4. Components

### 4.1 `synapsed` — daemon
- **Lang:** Rust (tokio, rusqlite, fastembed-rs)
- **Socket:** `$XDG_RUNTIME_DIR/synapse.sock` (unix domain, 0600)
- **Wire:** msgpack-rpc; methods: `put`, `put_batch`, `search_lex`, `search_vec`, `search_hybrid`, `ask`, `timeline`, `snap`, `restore`, `stats`
- **Persistence:** SQLite in WAL mode, `synchronous=NORMAL`, `journal_size_limit=64MB`
- **Startup:** <50ms (no Python, no ONNX session init until first embed call)
- **Memory floor:** ~20MB idle (Rust + SQLite handles)

### 4.2 Embedding pipeline
- **Model:** BGE-small-en-v1.5 (384-dim, ONNX, ~130MB) — matches SuperKnow v2
- **Runtime:** `fastembed-rs` w/ ONNX Runtime → Apple ANE via CoreML EP on macOS
- **Batch size:** 256 (config); queue flushes every 10ms OR on batch-full
- **Dedup:** BLAKE3(text) lookup in `redb` cache before embed → identical text costs 0
- **Cold embed cost:** ~90ms first call (ANE warmup), <2ms/doc amortized after

### 4.3 Storage schema
```sql
CREATE TABLE docs (
  id INTEGER PRIMARY KEY,
  uri TEXT UNIQUE,
  title TEXT,
  text TEXT NOT NULL,          -- stored compressed via zstd if >1KB
  meta BLOB,                   -- msgpack metadata
  ts INTEGER NOT NULL,         -- unix ms
  blake3 BLOB UNIQUE           -- 32 bytes, for dedup
);
CREATE VIRTUAL TABLE docs_fts USING fts5(
  title, text, content='docs', content_rowid='id',
  tokenize='porter unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE docs_vec USING vec0(
  id INTEGER PRIMARY KEY,
  embedding FLOAT[384]
);
-- triggers keep fts + vec in sync with docs
```

### 4.4 Query planner
- **Lex only:** `FTS5 MATCH` → ~1ms on 1M rows
- **Vec only:** `sqlite-vec KNN` → ~6ms on 1M rows
- **Hybrid (RRF):** parallel FTS5 + vec, reciprocal-rank-fusion at alpha=0.5, ~8ms
- **Time-filtered:** FTS5/vec → SQL `WHERE ts BETWEEN ?` on primary index
- **Ask (synthesis):** top-k retrieve → stream to local LLM (gemma3:270m default, configurable)

### 4.5 `.brainpack` format
- Header: 32 bytes — magic `BPK1`, version, zstd-level, uncompressed size
- Body: `zstd(sqlite-file)` — the whole DB as one stream
- Footer: BLAKE3 checksum, 32 bytes
- That's it. No WAL, no indexes separately — all inside the SQLite snapshot.
- Verify: `synapse verify foo.brainpack` → hash check + open-test.

### 4.6 Client SDKs
- **Rust:** `synapse-core` crate (daemon + in-proc modes share the impl)
- **Node:** `@synapse/sdk` — msgpack over unix socket, <5 KB
- **Python:** `synapse-py` — same, ships binary ext for msgpack-rpc perf
- **CLI:** `synapse` single binary — wraps the Rust SDK

## 5. Kill-Features vs MV2

1. **Daemon-mode.** No CLI spawn = -200ms/call. Biggest single win.
2. **Batch embedding.** 256-doc ANE batches = ~50× single-call throughput.
3. **BLAKE3-dedup before embed.** Identical text → zero compute.
4. **FTS5 live triggers.** No rebuild phases. MV2 rebuilds Tantivy on segment commits.
5. **sqlite-vec HNSW.** 6ms p95 verified on 2.4M rows (bench 2026-04-19).
6. **CRDT layer (optional, v2).** yrs on metadata → merge-able brains, team sync without locks.
7. **DuckDB ATTACH** works out of the box → analytics without copy.
8. **litestream replication** optional → continuous S3 backup, point-in-time restore.
9. **MCP-endpoint mode.** `synapse mcp` exposes the daemon as MCP server, direct Claude/agent tool-call.
10. **Column-store export.** `synapse export --format parquet` for ML downstream.

## 6. Benchmark Harness

Reuse `bench_v2.sh` structure (proven against MV2). Workloads:
- `insert-small` — 1k docs, 30-word Lorem
- `insert-large` — 100k docs, mixed lengths (markdown, code, prose)
- `query-lex` — 100 queries, warm cache
- `query-vec` — 100 queries, 10-NN
- `query-hybrid` — 100 queries, RRF fusion
- `cold-start` — import+first-query (measures daemon warmup)
- `export-import` — roundtrip 10k-doc DB through `.brainpack`

Baseline competitors: MV2 (memvid-cli), plain SQLite+FTS5, plain SQLite+sqlite-vec, Qdrant+local-embed.

## 7. Milestones

| # | Milestone | Deliverable | Time | Status |
|---|---|---|---|---|
| M0 | Repo + masterplan | this document | ✓ | DONE |
| M1 | `synapse-core` crate — SQLite schema, FTS5, sqlite-vec | Rust lib, unit tests | 1d | TODO |
| M2 | Embedding pipeline — fastembed-rs + BLAKE3 dedup + batch queue | `embed(Vec<String>) -> Vec<Vec<f32>>` | 1d | TODO |
| M3 | `synapsed` daemon — unix socket, msgpack, 10 methods | daemon binary | 1.5d | TODO |
| M4 | CLI + Node SDK parity with memvid-cli | `synapse` binary + npm pkg | 1d | TODO |
| M5 | `.brainpack` export/import | round-trip tests | 0.5d | TODO |
| M6 | Bench harness vs MV2/SQLite/Qdrant | numbers in README | 0.5d | TODO |
| M7 | MCP mode, litestream hook, CRDT metadata | v0.2 | 2d | stretch |

**MVP (M1-M6) = 5.5 days** of focused work.

## 8. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| sqlite-vec maturity (v0.1.x) | pin version, have tantivy fallback path in query planner |
| ANE availability off-macOS | fall back to ONNX CPU; BGE-small runs ~50ms CPU single |
| Daemon crash loses in-flight queue | fsync WAL every N inserts; queue journaled to disk |
| Schema migrations | schema version in `meta` table + idempotent `up_to(version)` migrator |
| MV2 ecosystem moves faster | focus on speed + MCP, not feature-parity |

## 9. Why This Wins

- SQLite is battle-tested. MV2 reinvents storage — we don't.
- Every tool in the Supersynergy stack (DuckDB, Polars, Datafusion, litestream) speaks SQLite natively. `.mv2` speaks only itself.
- Daemon-mode eliminates 80% of MV2's slowness with zero format innovation.
- Reuses bench-verified components already on the machine (`fastembed BGE-small`, `sqlite-vec`, `fastembed-rs`). No new dependencies to evaluate.
- Portability is free: `.backup` + zstd. MV2's whole format exists to solve a problem SQLite already solved.

## 10. Open Questions

- [ ] Should `.brainpack` include an optional *readable* sidecar (`manifest.toml`) for `git diff` legibility, at the cost of "one file" purity?
- [ ] CRDT layer as v1 or v2? Yrs adds ~400 KB binary size and some complexity.
- [ ] Default embedding model: BGE-small (speed) vs nomic-embed-text (accuracy)? Current call: BGE-small to match SuperKnow v2.
- [ ] Expose raw SQLite read-handle for power users, or enforce daemon-only access?
- [ ] Licensing: MIT (current) vs dual AGPL+commercial for future hosted service?

## 11. Non-Stack (explicit no-gos)

Per Supersynergy stack policy: no Qdrant dependency (we ARE the vector store), no Python runtime in daemon (ONNX-rs only), no n8n / Webpack / Puppeteer / Selenium / pip. Docker is optional for deploy, not for dev.

---

**Next action:** scaffold `synapse-core` crate, land M1 schema + FTS5 + sqlite-vec integration with one failing bench test green.
