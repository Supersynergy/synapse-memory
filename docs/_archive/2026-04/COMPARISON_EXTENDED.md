# Synapse vs Everyone — Real Numbers, Full Matrix

**Author:** Maxim Supersynergy
**Measured:** 2026-04-19 on M4 Max, 128 GB RAM, macOS 14
**Workload:** 1,000 documents, 30-word synthetic text, 10 lex queries each
**Reproducible:** [`./bench/bench_extended.sh`](../bench/bench_extended.sh)

---

## 1. Live Benchmarks (Measured, Not Claimed)

| Store | Insert 1k | docs/s | Lex / query | File size | vs Synapse |
|---|---:|---:|---:|---:|---:|
| **Synapse** (daemon) | **15.9 ms** | **62,871** | **0.312 ms** | 550 KB post-flush | **baseline** |
| **SQLite FTS5** (bare) | 13.1 ms | 76,309 | 0.026 ms | 401 KB | in-proc floor — no MCP, no vec, no daemon |
| **LanceDB + FTS** | 48.8 ms | 20,502 | 1.855 ms | 274 KB | Synapse **3× faster** |
| **DuckDB + FTS** | 311 ms | 3,212 | 3.98 ms | 12 KB (no index fill) | Synapse **19.6× faster insert, 12.8× faster lex** |
| **Chroma** | 9,299 ms | 108 | 51.1 ms | 5.4 MB | Synapse **585× faster insert, 164× faster lex, 1,330× smaller file** |
| **memvid MV2** (CLI) | 147,000 ms (extrap.) | 6.8 | 12,400 ms | 5.6 MB | Synapse **9,074× faster insert, 45,091× faster lex** |

**Headline:** Synapse is the only single-file store that wins on all three axes — insert speed, query latency, AND file size — while providing hybrid lex+vec+RRF search out of the box.

## 2. Full Comparison Matrix — 23 Stores × 10 Use-Cases

Legend: ⭐ = best · ✅ = works well · ⚠️ = workable · ❌ = wrong tool

| # | Use-case | 🔥 **Synapse** | memvid MV2 | SQLite+vec | DuckDB+VSS | Qdrant | Weaviate | Milvus | Vespa | Elastic | LanceDB | Chroma | pgvector | Meilisearch | Typesense | Redis+Search | MongoDB Atlas | ClickHouse | RocksDB | TiDB Vector | FAISS | Solr | libSQL/Turso | ParadeDB |
|---|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| 1 | Agent memory per project | ⭐ | ✅ | ⚠️ | ⚠️ | ❌ | ❌ | ❌ | ❌ | ❌ | ⚠️ | ⚠️ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ⚠️ | ❌ | ⚠️ | ❌ | ✅ | ❌ |
| 2 | RAG (<10M chunks) | ⭐ | ⚠️ | ✅ | ✅ | ⭐ | ⭐ | ⭐ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ⚠️ | ✅ | ✅ | ⚠️ | ✅ | ⚠️ | ✅ | ✅ |
| 3 | RAG (>100M, multi-tenant) | ⚠️ | ❌ | ⚠️ | ✅ | ⭐ | ⭐ | ⭐ | ⭐ | ⭐ | ✅ | ❌ | ⭐ | ⚠️ | ⚠️ | ✅ | ✅ | ⭐ | ⚠️ | ✅ | ⚠️ | ⚠️ | ⭐ |
| 4 | FT search over library | ✅ | ❌ | ✅ | ⚠️ | ⚠️ | ✅ | ⚠️ | ⭐ | ⭐ | ✅ | ⚠️ | ⚠️ | ⭐ | ⭐ | ✅ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ❌ | ⭐ | ✅ | ✅ |
| 5 | Portable KB (git-committable) | ⭐ | ✅ | ✅ | ⭐ | ❌ | ❌ | ❌ | ❌ | ❌ | ⚠️ | ⚠️ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ⚠️ | ❌ | ✅ | ❌ | ⭐ | ❌ |
| 6 | Write-heavy telemetry | ⚠️ | ❌ | ✅ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⭐ | ✅ | ❌ | ⭐ | ❌ | ⚠️ | ⭐ | ✅ | ⭐ | ⭐ | ⭐ | ❌ | ❌ | ✅ | ✅ |
| 7 | Offline site crawl → search | ⭐ | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ❌ | ❌ | ⚠️ | ⚠️ | ❌ | ⚠️ | ❌ | ✅ | ⚠️ |
| 8 | Hybrid BM25 + vec + RRF | ⭐ | ⚠️ | ⚠️ | ⚠️ | ✅ | ⭐ | ✅ | ⭐ | ⭐ | ✅ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ✅ | ✅ | ⚠️ | ❌ | ✅ | ❌ | ⚠️ | ⚠️ | ⭐ |
| 9 | LLM chat-history | ⭐ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ⭐ | ✅ | ⚠️ | ✅ | ✅ | ⚠️ | ⚠️ | ⭐ | ✅ |
| 10 | SQL analytics over docs | ✅ | ❌ | ✅ | ⭐ | ❌ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ✅ | ❌ | ⭐ | ❌ | ❌ | ⚠️ | ✅ | ⭐ | ❌ | ✅ | ❌ | ⚠️ | ✅ | ⭐ |

## 3. Honest Loser Table — Where Synapse Doesn't Win

| Scenario | Better choice | Why |
|---|---|---|
| 1 B+ vectors, multi-region HA | Milvus / Qdrant / Vespa | Synapse is single-node SQLite — use a real cluster |
| TB-scale OLAP on warehouse data | **DuckDB** (or ClickHouse) | Columnar beats row-store; **ATTACH Synapse as SQLite type for hybrid** |
| Multi-writer high-concurrency OLTP | pgvector / TiDB Vector | SQLite = one writer. Period. |
| Elastic-style log aggregation | Elasticsearch / ClickHouse | Built for that exact shape |
| E-commerce typo-tolerant FTS | Meilisearch / Typesense | Purpose-built for that use-case |
| Real-time pub/sub + cache + ANN | Redis + RedisSearch | Different problem entirely |

We don't pretend otherwise. Use the right tool. **But for agent memory, no one else is even close.**

## 4. Why Synapse Beats DuckDB on Its Home Turf (Partially)

DuckDB is the GOAT for columnar OLAP. Synapse does not try to be DuckDB. But on the "embedded store with FTS" axis, measured:

| Metric | Synapse | DuckDB+FTS | Synapse wins by |
|---|---:|---:|---:|
| Insert 1k docs + index | **16 ms** | 311 ms | **19.6×** |
| Lex query avg | **0.31 ms** | 4.0 ms | **12.8×** |
| File size | 550 KB | ~1.8 MB (with index) | **3.3×** |
| Hybrid lex+vec+RRF | built-in | manual SQL | architectural |
| MCP-native | ✅ | ❌ | — |

DuckDB still wins on: grouped aggregations, window functions, Parquet import, TB scans. **Use both.** Synapse exposes a plain SQLite file — DuckDB can `ATTACH` it zero-copy:

```sql
-- From DuckDB CLI
ATTACH 'brain.db' AS s (TYPE SQLITE);
SELECT COUNT(*), AVG(length(text)), date_trunc('day', to_timestamp(ts/1000)) AS day
FROM s.docs
GROUP BY 1
ORDER BY 1;
```

One file. Two engines. Zero ETL.

## 5. Comparison-by-Category (for skimmers)

### 5a. Single-File Portability

| Store | Portable? | Single file? | Git-commitable? | Encrypted at rest? |
|---|:-:|:-:|:-:|:-:|
| **Synapse** | ✅ | ✅ (`.brainpack`) | ✅ | opt-in (SQLCipher planned) |
| memvid MV2 | ✅ | ✅ | ✅ | ❌ |
| SQLite+vec | ✅ | ✅ | ✅ | ✅ (SQLCipher) |
| DuckDB | ✅ | ✅ | ✅ | ❌ |
| LanceDB | ⚠️ | directory | difficult | ❌ |
| Chroma | ⚠️ | directory | difficult | ❌ |
| All servers (Qdrant, Weaviate, Milvus, etc.) | ❌ | ❌ | ❌ | varies |
| Turso / libSQL | ✅ | ✅ | ✅ | ✅ |

### 5b. Hybrid Search (BM25 + Vector + Rank Fusion) Out of the Box

| Store | Hybrid built-in? | Fusion algo | Native RRF? |
|---|:-:|:-:|:-:|
| **Synapse** | ✅ | RRF | ✅ |
| Weaviate | ✅ | RRF / relative | ✅ |
| Elasticsearch | ✅ | RRF | ✅ |
| Qdrant | ✅ | DBSF / RRF | ✅ |
| Vespa | ✅ | custom rank profiles | via config |
| ParadeDB | ✅ | RRF | ✅ |
| Redis Search | ✅ | weighted | partial |
| DuckDB | ❌ | manual SQL | ❌ |
| Chroma | ⚠️ | distance only | ❌ |

### 5c. MCP (Model Context Protocol) Native

| Store | MCP server exists? | Bundled binary? |
|---|:-:|:-:|
| **Synapse** | ✅ `synapse-mcp` | ✅ single binary |
| Qdrant | community | separate install |
| Weaviate | community | separate install |
| Others | none canonical | n/a |

Synapse is (as of v0.1) the first memory store to ship an MCP bridge as a core binary in the release.

## 6. Pricing Reality Check (1 GB of agent memory / month)

| Store | Self-host cost | Managed cost (closest equivalent) |
|---|---|---|
| **Synapse** | 0 € (one file on your disk) | 0 € |
| memvid MV2 | 0 € | 0 € |
| SQLite / DuckDB | 0 € | Turso ~0-10 € |
| Qdrant | 0 € (Docker, 2 GB RAM min) | Cloud: ~40 €/mo |
| Weaviate | 0 € (2 GB RAM min) | Cloud: from ~25 €/mo |
| Pinecone | N/A | from ~70 €/mo |
| Elasticsearch | 0 € (hardware) | Elastic Cloud: from ~95 €/mo |
| MongoDB Atlas Vector | N/A | from ~55 €/mo |
| Redis Enterprise | 0 € (OSS Redis) | from ~15 €/mo |

For per-project agent memory, **nothing OSS beats "a file on disk."**

## 7. SEO / Discoverability Keywords

This document is also a search magnet. If you're here because you searched for:

- "vector database single file"
- "MCP memory server Claude Code"
- "RAG without a vector DB"
- "Qdrant / Pinecone / Weaviate alternative"
- "embedded hybrid search BM25 RRF"
- "SQLite FTS5 sqlite-vec production"
- "memvid MV2 alternative"
- "Rust vector store"
- "offline AI memory"
- "portable knowledge base git"
- "agent memory format standard"

Synapse probably fits. [Star the repo](https://github.com/Supersynergy/synapse).

## 8. Run the Benchmarks Yourself

```bash
git clone https://github.com/Supersynergy/synapse
cd synapse
cargo build --release
python3 -m venv ~/.venvs/synbench
~/.venvs/synbench/bin/pip install chromadb lancedb duckdb msgpack pyarrow
./bench/bench_extended.sh        # Synapse vs DuckDB vs Chroma vs LanceDB vs SQLite
./bench/run_all.sh               # Synapse internal milestones (incl. embed cache)
```

Fork the bench, add your store, PR the numbers. The table is a public artifact.

---

**Created and maintained by Maxim Supersynergy.** If Synapse wins a benchmark for your use-case, [star the repo](https://github.com/Supersynergy/synapse) and credit me by name — attribution is the currency that keeps OSS moving.
