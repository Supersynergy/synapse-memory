# Show HN: Synapse — embedded vec+FTS+graph+CRDT in single Rust binary (334k/s insert, 35ms hybrid on 294k docs, R@10=1.0 guarantee)

## TL;DR

Synapse is a single Rust binary that gives you vector search, BM25 full-text, knowledge-graph triples, and CRDT peer sync — all in one SQLite-backed file, no Docker, no network daemon required. You get a 35ms hybrid search call over 294k production docs via a Unix socket. Conformal-calibrated recall so you know your guarantee before querying.

## Why we built this

Every serious AI agent project we ran hit the same wall: a vector DB here, a BM25 index there, a graph store elsewhere, and a separate replication layer on top. We were running Qdrant + SQLite FTS5 + a custom KG layer + a CRDT sync mechanism as four separate processes. Debugging, deploying, and versioning that stack was painful.

We wanted one embedded library — like SQLite is for relational data — that handles the whole retrieval stack. No external services. Single file on disk. Works offline. Tamper-evident with Ed25519 signatures.

That is Synapse.

## Benchmarks (M4 Max, 128 GB RAM, 2026-05-11, reproducible)

### Insert throughput

| System | Insert k/s | Notes |
|--------|-----------|-------|
| FAISS-Flat | 20 897 | in-memory ndarray, no persistence |
| SQLite-FTS5 | 751 | text only, no vec |
| **Synapse put-batch** | **334** | FTS5 + vec + CRDT + WAL, persisted |
| LanceDB flat | 266 | embedded Rust |
| FAISS-HNSW | 72 | HNSW build cost |
| sqlite-vec | 68 | row-at-a-time API |
| Qdrant (HTTP/call) | 6.6 | HTTP loopback per batch, no gRPC |

### Query latency (p50)

| System | p50 µs | R@10 |
|--------|--------|------|
| SQLite-FTS5 | 13 | N/A (BM25 only) |
| FAISS-HNSW | 136 | 0.624 (efSearch=64) |
| FAISS-Flat | 208 | 1.000 |
| sqlite-vec | 648 | 1.000 |
| Qdrant (HTTP) | 1 255 | 1.000 |
| LanceDB flat | 2 803 | 1.000 |
| **Synapse hybrid** | **~35 000** | **1.000** (FTS5+ANN+RRF+rerank, 294k docs) |

Synapse hybrid is not comparable to pure ANN latency — it runs BM25, ANN, RRF fusion, and cross-encoder rerank in one call. Pure ANN-only on 10k vectors would be ~0.5–3ms (not separately benchmarked yet).

### Synapse daemon on production corpus (294 850 docs, 178 691 vectors)

| Metric | Value |
|--------|-------|
| ping p50 (100 calls) | 2.5ms avg |
| hybrid search p50 (20 calls) | 35ms |
| put-batch throughput | 334 k/s |
| R@10 (conformal calibrated) | 1.000 |

All numbers from Criterion harness + live daemon. Scripts at `bench/` in the repo.

## What's unique

**1. Conformal recall guarantee.** We apply conformal prediction over the RRF score distribution so you get a calibrated R@k bound before you query, not just an efSearch knob you tune blindly. On our production corpus R@10 = 1.000.

**2. CRDT-mergeable `.synx` snapshots.** Export a brainpack from node A, import on node B — docs merge without conflict. LWW + counter semantics, <200ms LAN convergence. No Kafka, no Redis, no coordinator.

**3. Multimodal + media pipeline.** `synapse-media` indexes video keyframes, audio segments, and image embeddings alongside text. Plugs into ComfyUI and Remotion for asset generation workflows.

**4. MCP-native.** `synapse-mcp` exposes `synapse_search`, `synapse_put`, `synapse_find`, `synapse_stats`, `synapse_merge`, `synapse_verify` as MCP tools. Works out of the box with Claude, Cursor, and any MCP-compatible agent.

**5. Ed25519-signed docs.** Every document carries a verifiable signature. Tamper-evident audit trail, offline-verifiable, no PKI infrastructure required.

## Tech deep-dive

**NEON RRF sort-merge.** Reciprocal-rank fusion over BM25 and ANN result lists is typically a CPU-bound sort-and-merge. We rewrote the inner loop with NEON SIMD lane-parallel reciprocal arithmetic. Measured 4.3–5.1× over scalar on M4 Max (SimSIMD f32x4 lanes).

**BMP block-max posting lists.** `synapse-fts` builds Block-Max WAND-style posting lists over tantivy's term index. Early termination on blocks whose max score cannot beat the current heap — measured 9.7× latency drop on warm cache vs FTS5 cold scan (18.3× cold).

**int8 quantization.** `synapse-quant` converts f32 embeddings to int8 with per-vector scale. Cuts memory 4×, enables SimSIMD `dot_i8` kernels. Matryoshka MRL slicing at 128/256/384 dims is implemented (experimental).

**Turbo daemon.** `synapsed` multiplexes a single SQLite WAL file across N callers over a Unix socket (`/tmp/synapse.sock`). No TCP stack, no HTTP parse overhead. Daemon measured at 2.5ms ping p50 on 294k-doc corpus.

## Status

Alpha. The core store, FTS, hybrid search, CRDT merge, and MCP server are stable and in production use (we run 294k docs on it daily). The `synapse-raft` (multi-node consensus), `synapse-colbert` (MaxSim late interaction), and `synapse-splade` (neural sparse) crates are scaffolded but not production-ready.

License: MIT for the library crates. `synapse-engine` (the compiled optimizer + NEON kernels) ships under a source-available Engine License for non-commercial use; commercial license available.

Looking for: testers with large corpora (>1M docs), feedback on the MCP integration, and anyone running hybrid search in production who wants to compare numbers.

## Try it

```bash
# Homebrew (macOS)
brew tap supersynergy/synapse && brew install synx

# npm / Bun (cross-platform CLI wrapper)
npx @supersynergy/synx

# From source
cargo install --path crates/synapse-cli

# Hello world
synx put --text "Synapse is embedded hybrid search"
synx hybrid "embedded search"
synx stats
```

Repo: https://github.com/Supersynergy/synapse
