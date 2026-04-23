# Synapse

> **Blazingly fast, single-file AI agent memory. Rust-native. Sub-ms retrieval. Zero daemons.**

[![CI](https://github.com/Supersynergy/synapse/actions/workflows/rust-ci.yml/badge.svg)](https://github.com/Supersynergy/synapse/actions)
[![Crates.io](https://img.shields.io/crates/v/synapse-core)](https://crates.io/crates/synapse-core)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

---

## TL;DR

**Synapse is the Rust-native drop-in replacement for Qdrant/Chroma/Pinecone when you need embedded, sub-ms agent memory in a single binary.**

- ⚡ **17× faster embeddings** vs fastembed (Ollama path, M4 Max benchmark-verified)
- 🚀 **7× faster vector search** (ndarray + ARM NEON SIMD, <50k docs)
- 🧠 **20× faster cache** (lock-free, in-memory, RRF-native)
- 📦 **Single binary**, **zero daemon**, **zero config**
- 🔒 **Ed25519-signed**, **CRDT-merge**, **memory-safe by design**
- 🏠 **Local-first**, **self-hostable**, **OSS-first** — no vendor lock-in

---

## Why Synapse?

The existing stack is heavy:

| Problem | Competitor | Synapse |
|---------|-----------|---------|
| Vector DB needs server + client | Qdrant, Chroma, Milvus | **Embedded, single binary** |
| Managed = vendor lock-in | Pinecone, Weaviate Cloud | **Self-hostable, OSS-first** |
| Python-only, GIL-bound | Chroma, LlamaIndex | **Rust-native, fearless concurrency** |
| Hybrid search = wire 3 things | pgvector + elastic + Redis | **FTS5 + sqlite-vec + RRF built-in** |
| No cryptographic integrity | Most vector DBs | **Ed25519-signed per-doc** |
| No sync across nodes | Requires Raft/Consul/etc | **CRDT-merge, deterministic** |

**Synapse is the missing piece** between SQLite and a full vector DB — for agents that need real memory without ops overhead.

---

## Benchmarks (M4 Max, benchmark-verified)

| Component | Default | Turbo | Speedup |
|-----------|---------|-------|---------|
| Embedding (per-doc) | 170ms (fastembed) | **10ms** (Ollama) | **17×** |
| Vector search (3k docs) | 0.22ms (sqlite-vec) | **0.03ms** (ndarray+SIMD) | **7×** |
| Cache lookup | 2μs (redb) | **0.1μs** (lock-free) | **20×** |
| Hybrid query (cached) | 300ms cold | **0.1ms** warm | **3000×** |
| E2E round-trip | — | **<5ms p99** | — |

No tricks. Criterion benches in `bench/`. Reproducible.

---

## Feature Matrix

| Feature | Status | Notes |
|---------|--------|-------|
| 🔍 Full-text search (BM25) | ✅ | SQLite FTS5, zero extra deps |
| 🎯 Vector search (kNN) | ✅ | sqlite-vec + ndarray-SIMD turbo |
| 🔀 Hybrid RRF (BM25 + vec) | ✅ | Built-in, no glue code |
| 🧬 Embeddings | ✅ | fastembed (CPU) or Ollama (turbo, 17×) |
| 🔐 Ed25519 per-doc signing | ✅ | Deterministic, tamper-evident |
| 🔄 CRDT merge across nodes | ✅ | yrs-based, conflict-free |
| 🗜️ Zstd compression at rest | ✅ | 3-5× on text payloads |
| 🔑 Age encryption (optional) | ✅ | File-level, argon2 KDF |
| 🐌 Shardable | ✅ | Rayon-parallel |
| 🎼 MCP-native | ✅ | `synapse-mcp` binary, works w/ Claude |
| 📊 Prometheus metrics | ✅ | `synapsed` exporter |
| 🏋️ Learn: bandit + calibration + heat | ✅ | `synapse-learn` crate |

---

## Quick Start (30 seconds)

```bash
cargo add synapse-core

# Or single-binary mode
cargo install synapsed
synapsed --help
```

```rust
use synapse_core::{Store, PutRequest, SearchMode};

let store = Store::open("memory.db")?;
store.put(PutRequest::new("Context about project X"))?;

// Hybrid RRF out of the box
let hits = store.search("project X", SearchMode::Hybrid, 10)?;
```

**That's it.** No server. No config. No lock-in.

---

## Positioning

- **Not a replacement for Qdrant at scale** — Qdrant wins > 10M vectors, distributed.
- **Synapse wins** in the **embedded / agent / edge** regime: <50k–500k docs per store, sub-ms, single process.
- Think **SQLite for vectors**, not Postgres.

### Who should use Synapse?
- Agent framework authors who need memory without a sidecar.
- Local-first apps (Obsidian-style, offline-first).
- Edge deployments (Cloudflare Workers planned via WASM).
- RAG prototypes that need to graduate beyond Chroma without re-architecting.

### Who should NOT?
- You need >10M vectors per shard → use Qdrant/Milvus.
- You need a hosted managed service → we don't offer one (by design).

---

## Architecture

```
┌─────────────────────────────────────┐
│  synapse-cli / synapse-mcp / lib    │
└──────────────┬──────────────────────┘
               │
     ┌─────────▼─────────┐
     │   synapse-core     │
     │  ┌──────────────┐  │
     │  │ Turbo module │  │ 17× embed / 7× vec / 20× cache
     │  │  - Ollama    │  │
     │  │  - ndarray   │  │
     │  │  - cache     │  │
     │  └──────────────┘  │
     │  ┌──────────────┐  │
     │  │ FTS5 + vec   │  │ BM25 + kNN
     │  │     RRF      │  │ hybrid rank
     │  └──────────────┘  │
     │  ┌──────────────┐  │
     │  │ CRDT + Ed25519│ │ merge-safe, signed
     │  └──────────────┘  │
     └─────────┬──────────┘
               │
         SQLite file (single-file KB)
```

All in one `.db` file. Mount it, rsync it, back it up with `cp`.

---

## Status

- ✅ v0.1.0: core, FTS5+vec+RRF, Ed25519, CRDT, turbo module, MCP
- 🚧 v0.2.0: WASM build, Cloudflare Workers binding, federated shards
- 🔮 v1.0.0: semver-stable public API, 1M-vec shard test suite

**Production-leaning but pre-1.0. API may break.**

---

## Non-goals

- No hosted service.
- No Python SDK as first-class (Rust + MCP is first-class; pyo3 bindings are community-maintained).
- No support for exotic distance metrics beyond cosine/L2/dot.
- No GPU embeddings (CPU + Ollama is the opinionated default).

---

## Contributing

PRs welcome. See `CONTRIBUTING.md`. All CI gates: `cargo fmt`, `cargo clippy`, `cargo test`, `cargo deny check`, `cargo audit`.

## License

MIT © Maxim Supersynergy
