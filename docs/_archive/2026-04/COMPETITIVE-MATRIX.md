# Synapse Competitive Matrix — Top 50 Memory / Vector / KG Stores

**Date**: 2026-05-04
**Method**: Synapse memory (`synx hybrid`), `ghgrep` searches (`ai memory mcp`, `claude memory persistent`, `vector database embedded rust`, `agent memory framework`), known-set from training + project memory.
**Honesty**: Capabilities marked `?` mean unverified from primary source in this pass — do not cite as fact.
**Columns**: Stack | License | Vector | FTS | KG | CRDT | Sign | Persist | Local-first | MCP | Distinct

Legend: ✅ first-class · 🟡 partial / via-extension · ❌ none · ? unverified

Sort: rust-native first, then local-first embedded, then MCP-native, then cloud/SaaS at end.

| # | Project | Stack | License | Vec | FTS | KG | CRDT | Sign | Persist | Local-first | MCP | Distinct |
|---|---------|-------|---------|-----|-----|----|----|------|---------|-------------|-----|----------|
| 1 | **Synapse** (this) | Rust+SQLite+sqlite-vec+yrs+ed25519 | MIT | ✅ | ✅ FTS5+Tantivy | ✅ triples | ✅ yrs | ✅ ed25519 | ✅ .synx | ✅ | ✅ synapse-mcp | Single-binary 8-cap memory store on M4 Max; FTS p50 51µs |
| 2 | LanceDB | Rust+Arrow+Lance | Apache-2.0 | ✅ HNSW | 🟡 scalar | ❌ | ❌ | ❌ | ✅ Lance fmt | ✅ | 🟡 community | Versioned columnar+vector, ML lakehouse focus |
| 3 | Qdrant | Rust+RocksDB | Apache-2.0 | ✅ HNSW+quant | 🟡 payload-text | ❌ | ❌ | ❌ | ✅ disk | 🟡 server | 🟡 community | Distributed sharding, payload filters, gRPC API |
| 4 | sqlite-vec | C+SQLite ext | Apache-2.0/MIT | ✅ flat+brute | 🟡 via FTS5 sibling | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Tiny SQLite extension, ~1k LOC, embeddable everywhere |
| 5 | tantivy | Rust | MIT | ❌ | ✅ Lucene-style | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Rust Lucene; BM25 + facets, embed-friendly |
| 6 | Meilisearch | Rust+LMDB | MIT | 🟡 v1.6+ vec | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ self-host | ❌ | Typo-tolerant relevance, fast typo+prefix |
| 7 | redb | Rust pure | MIT/Apache | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Pure-Rust embedded KV, ACID, mmap |
| 8 | fjall | Rust pure | MIT/Apache | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ LSM | ✅ | ❌ | Pure-Rust LSM (RocksDB alt), no C deps |
| 9 | usearch | C++ (Rust binding) | Apache-2.0 | ✅ HNSW | ❌ | ❌ | ❌ | ❌ | ✅ sidecar | ✅ | ❌ | Smallest HNSW (single-header), used by Synapse-ann PR-A1 |
| 10 | hora | Rust pure | Apache-2.0 | ✅ HNSW/IVF/PQ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Pure-Rust ANN, multiple algos |
| 11 | instant-distance | Rust pure | Apache-2.0/MIT | ✅ HNSW | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Minimal HNSW, focused API |
| 12 | sled | Rust | Apache/MIT | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Lock-free embedded KV (maintenance mode) |
| 13 | DuckDB + vss | C++ (Rust binding) | MIT | ✅ HNSW (vss ext) | ✅ FTS ext | ❌ | ❌ | ❌ | ✅ | ✅ | 🟡 community | OLAP analytics + vector in-process |
| 14 | libSQL / Turso | C+Rust+SQLite | MIT | 🟡 native vec | ✅ FTS5 | ❌ | ❌ | ❌ | ✅+edge | ✅ | ❌ | SQLite fork with native sync to edge |
| 15 | pgvector | C (Postgres ext) | PostgreSQL | ✅ HNSW/IVF | ✅ tsvector | 🟡 via Apache AGE | ❌ | ❌ | ✅ pg | 🟡 server | 🟡 community | Postgres ecosystem leverage |
| 16 | Marqo | Python+Vespa+ONNX | Apache-2.0 | ✅ | ✅ | ❌ | ❌ | ❌ | 🟡 docker | 🟡 self-host | ❌ | End-to-end multimodal vector search |
| 17 | Vespa | Java+C++ | Apache-2.0 | ✅ | ✅ | 🟡 tensor | ❌ | ❌ | ✅ | ❌ | ❌ | Tensor + lexical, ranking expressions |
| 18 | Milvus | Go+C++ | Apache-2.0 | ✅ many idx | 🟡 scalar | ❌ | ❌ | ❌ | ✅ standalone | 🟡 community | 🟡 community | Cloud-native distributed vector DB |
| 19 | Vald | Go+NGT | Apache-2.0 | ✅ NGT | ❌ | ❌ | ❌ | ❌ | 🟡 k8s | ❌ | ❌ | k8s-native cloud vector DB |
| 20 | Weaviate | Go | BSD-3 | ✅ | ✅ BM25 | ✅ schema-graph | ❌ | ❌ | 🟡 docker | 🟡 self-host | 🟡 community | Module ecosystem (rerankers, transformers) |
| 21 | Typesense | C++ | GPL-3 | ✅ v0.25+ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ self-host | ❌ | Algolia-alt; instant search UX |
| 22 | ChromaDB | Python+SQLite/DuckDB | Apache-2.0 | ✅ HNSW | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | 🟡 community | Default for python-LLM tutorials; **slow** vs Synapse 7.7× |
| 23 | FAISS | C++ | MIT | ✅ ref impl | ❌ | ❌ | ❌ | ❌ | 🟡 manual | ✅ | ❌ | Theoretical-floor ANN library, no DB layer |
| 24 | hnswlib | C++ (py binding) | Apache-2.0 | ✅ HNSW | ❌ | ❌ | ❌ | ❌ | 🟡 manual | ✅ | ❌ | Original HNSW reference impl |
| 25 | annoy | C++ (py binding) | Apache-2.0 | ✅ trees | ❌ | ❌ | ❌ | ❌ | ✅ mmap | ✅ | ❌ | Spotify ANN, mmap-friendly |
| 26 | ScaNN | C++ | Apache-2.0 | ✅ ScaNN algo | ❌ | ❌ | ❌ | ❌ | 🟡 | ✅ | ❌ | Google research-grade ANN, anisotropic quant |
| 27 | Vectra | TS pure | MIT | ✅ JSON disk | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Local-only TS vector DB (Pinecone-like API) |
| 28 | mem0 | Python | Apache-2.0 | ✅ via vec backend | 🟡 | ✅ entity | ❌ | ❌ | 🟡 user-store | 🟡 | ✅ | Agent memory layer abstraction (LLM-extracted facts) |
| 29 | Letta (MemGPT) | Python | Apache-2.0 | ✅ | 🟡 | 🟡 | ❌ | ❌ | 🟡 pg/sqlite | 🟡 | ✅ | OS-style virtual memory paging for LLM context |
| 30 | Zep | Go+Postgres | Apache-2.0 | ✅ pgvector | ✅ | ✅ Graphiti | ❌ | ❌ | 🟡 server | 🟡 | ✅ | Temporal KG agent memory; biotemporal facts |
| 31 | Graphiti | Python+Neo4j | Apache-2.0 | 🟡 via neo4j | 🟡 | ✅ temporal | ❌ | ❌ | 🟡 neo4j | 🟡 | ✅ | Temporal knowledge graph for agents |
| 32 | Cognee | Python | Apache-2.0 | ✅ | 🟡 | ✅ | ❌ | ❌ | 🟡 | 🟡 | ✅ | Pipeline-based memory (LLM extract → graph) |
| 33 | LangMem | Python | MIT | ✅ | 🟡 | 🟡 | ❌ | ❌ | 🟡 store | 🟡 | ✅ | LangChain memory primitives |
| 34 | Hindsight | Python | MIT | ✅ | 🟡 | ❌ | ❌ | ❌ | 🟡 | 🟡 | 🟡 | Browser-event agent memory |
| 35 | Supermemory | TS+Cloudflare | proprietary? | ✅ | 🟡 | ❌ | ❌ | ❌ | ❌ cloud | ❌ | ✅ | SaaS memory layer; CF Workers + Vectorize |
| 36 | Mastra | TS | Apache-2.0 | ✅ | 🟡 | 🟡 | ❌ | ❌ | 🟡 | 🟡 | ✅ | TS agent framework w/ memory primitives |
| 37 | claude-mem | TS | MIT | ? | ? | ? | ❌ | ❌ | ✅ | ✅ | ✅ | Claude-specific persistent memory MCP |
| 38 | cortex-mem | ? | ? | ? | ? | ? | ? | ? | ? | ? | ✅ | MCP memory server (unverified details) |
| 39 | omega-memory | Python | proprietary | ✅ | ✅ | ✅ link-graph | ❌ | ❌ | ✅ | ✅ | ✅ | Author-built persistent memory (this user's stack) |
| 40 | MemPalace | Python pipeline | research | ✅ | ✅ | ✅ | ❌ | ❌ | 🟡 | 🟡 | ❌ | Recall pipeline (LongMemEval baseline); R@5 ≥ Synapse pre-rerank |
| 41 | graphify | ? | ? | 🟡 | 🟡 | ✅ | ❌ | ❌ | ? | ? | 🟡 | KG-first agent memory (unverified specs) |
| 42 | sqlite-vss (legacy) | C+SQLite | Apache-2.0 | ✅ via FAISS | 🟡 | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Predecessor to sqlite-vec; FAISS-backed |
| 43 | DiskANN | C++ | MIT | ✅ disk-resident | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Microsoft graph-on-SSD ANN, billion-scale |
| 44 | SPANN | research | research | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Memory-disk hybrid ANN, Microsoft research |
| 45 | arroy | Rust pure | MIT | ✅ trees (LMDB) | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | Meilisearch's annoy-port on LMDB |
| 46 | ParlayANN | C++ | MIT | ✅ research-grade | ❌ | ❌ | ❌ | ❌ | 🟡 | ✅ | ❌ | Parallel HNSW/Vamana research baselines |
| 47 | glass-vector | ? | ? | ✅ ? | ❌ | ❌ | ❌ | ❌ | ? | ? | ❌ | Cited in ANN-bench; unverified |
| 48 | Pinecone | proprietary cloud | proprietary | ✅ | 🟡 | ❌ | ❌ | ❌ | ❌ | ❌ | 🟡 | SaaS reference; closed-source, lock-in |
| 49 | Cloudflare Vectorize | proprietary edge | proprietary | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ edge | ❌ | ❌ | Edge-native vector index; lock-in |
| 50 | Turbopuffer | proprietary cloud | proprietary | ✅ disk-cheap | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | Cheap object-storage-backed vector SaaS |

---

## Where Synapse wins

1. **Single binary, all 8 capabilities in one process.** No competitor in rows 2–50 ships Vector + FTS + KG + CRDT + signing + persistence + local-first + MCP from one Rust binary. Closest: Weaviate has Vec+FTS+KG-schema but is JVM/Go server. mem0/Letta have Vec+KG but rent vector backends and have no CRDT/signing.
2. **Embedded performance on M4 Max.** Rust criterion: FTS p50 = 51 µs, hybrid query = 0.023 ms/q, insert burst = 255k ops/s (batch=10k). 7.7× lower query latency than ChromaDB on identical workload (RESULTS.md MemPalace shootout).
3. **Zero external runtime.** No Docker, no JVM, no Python in hot path. Compare: Weaviate, Milvus, Qdrant, Vald all need server processes; ChromaDB/mem0/Letta need Python.
4. **Author-signing + CRDT.** ed25519 signing (sign.rs, 84 LOC real impl) + yrs CRDT (crdt.rs, 101 LOC) + federate.rs (462 LOC) — combination unmatched in rows 2–50. Audit-grade + multi-device merge axis nobody else covers.
5. **MCP-native.** synapse-mcp ships server with `synapse_search/put/find/stats`. Most rows 2–34 require community shims.
6. **Storage parity vs sqlite-vec, beats Chroma 3.4×.** 1290 KB vs Chroma 4434 KB at 1k docs (RESULTS.md row 17).

---

## Where Synapse loses

1. **Recall (pre-rerank) vs MemPalace pipeline.** Current LongMemEval R@5 = 0.30 (per-message). MemPalace and high-pipeline systems (Zep+Graphiti, Cognee) push 0.60+ via cross-encoder rerank + temporal KG fact extraction. Synapse-rerank exists (110 LOC, IdentityReranker + ONNX trait), not yet wired in eval path.
2. **Distributed scale.** No native sharding/replication. Qdrant, Milvus, Vald, Weaviate, Vespa win above ~100M-vector or multi-node workloads. synapse-seg + synapse-wal are stubs (18 LOC each, scaffold only).
3. **Ecosystem & tooling.** pgvector inherits the entire Postgres tooling universe (pgAdmin, Datagrip, replication, backup ops). Synapse has CLI + MCP + Python wheel — no GUI, no operator playbook yet.
4. **Python wheel adapter overhead.** RESULTS.md: query p50 regresses to 200 ms at 76k chunks via PyO3 adapter (vs 51 µs Rust direct). Fix = call `synapsed` RPC instead of inline FFI loop.
5. **ANN scale-out.** synapse-ann ships PR-A1 (UsearchIndex, 63 LOC trait + impl); IVF-PQ TODO. DiskANN, SPANN, ParlayANN, Milvus DiskANN backend dominate >10M vec.
6. **Cloud / managed offering.** None. Pinecone, Turbopuffer, CF Vectorize, Supermemory all serve "no-ops" market Synapse cannot.

---

## Sweet-spot positioning

Synapse is the **best answer** when ALL of the following hold:

- Single host (M-series Mac, edge box, dev laptop, AI workstation)
- ≤ 10M vectors (criterion 1k–100k validated; ann-bench hits 1M Sift @ parity)
- Need ≥ 4 of {vector, FTS, KG, CRDT, signing, MCP} in **one binary** with **no external services**
- Rust-or-Python consumer (PyO3 wheel; no JVM requirement)
- Local-first agent memory with audit-grade trail (signing) and multi-device merge (CRDT)

Concrete winning use cases:
- **Local Claude/Cursor/Aider agent memory** with persistence + cross-device sync via CRDT
- **Personal knowledge OS** (notes + chats + chunks + graph) on a laptop
- **Edge analytics box** that needs FTS+vec+KG without ops overhead
- **Audit-trail agent stores** where ed25519-signed memories are required (compliance, legal)
- **Synapsed-as-sidecar** for any TS/Rust/Python app that wants a local memory daemon over Unix-socket

When NOT to use Synapse: >100M vectors, multi-node sharding required, Postgres-ecosystem mandate, managed-SaaS preference, sub-1ms p99 at distributed scale.
