# synapsql roadmap — 8 weeks → SOTA

## P0 — wires (week 1)

- `synapsql-mysql`: opensrv-mysql v0.10 (Apache, async, Databend-tested)
- `synapsql-pg`: pgwire 0.40 (sunng87, async)
- `synapsql-grpc`: tonic + arrow-flight (push-down analytics)
- `synapsql-rest`: axum (REST + websockets, LiveQuery)
- `synapsql-core`: shared `Store` trait, EchoBackend → real path

**Gate**: 4 clients connect simultaneously to same Store, basic SELECT works.

## P1 — row + columnar (weeks 2-3)

- `synapsql-row`: libsql 0.9 async-WAL (concurrent writers, replication-ready)
- `synapsql-col`: Lance format (arrow-native, S3-tiered)
- `synapsql-fts`: SQLite FTS5 wrapper (extracted, 30µs latency)

**Gate**: TPC-C >700k QPS single-node, TPC-H within 2× DuckDB.

## P2 — exec engine (weeks 4-5)

- `synapsql-exec`: DataFusion 38 logical plan + custom physical
- `synapsql-jit`: Cranelift JIT (predicate eval, join probe hot loops)
- Vectorized batches (8192 rows)

**Gate**: ClickBench top-5 of all measured systems.

## P3 — consensus (week 6)

- `synapsql-raft`: openraft (databendlabs, 486 ghgrep hits)
- LiteFS-pattern page-shipping over raft log
- Optional: cr-sqlite CRDT for eventual edge nodes

**Gate**: 3-node raft cluster, <50ms commit p99.

## P4 — AI-native (week 7)

- `synapsql-ann`: usearch HNSW + ef-runtime knob + cascade rerank
- `synapsql-rerank`: ColBERT cross-encoder
- `synapsql-graph`: RELATE / Dijkstra (Synapse-graph port)
- `synapsql-quant`: i8 / MRL-128 / f16 / RaBitQ 1-bit
- `synapsql-metal`: MLX kernels (M4 Neural Engine)
- `synapsql-embed`: ort 2.0 EP cascade [CUDA, CoreML, CPU]
- SQL functions: `VECTOR_SEARCH(...)`, `RERANK(...)`, `GRAPH_HOP(...)`

**Gate**: recall@10 ≥ 0.99 on Sift-1M, 100k QPS hybrid.

## P5 — SaaS (week 8)

- `synapsql-tier`: object_store 0.11 (S3/GCS/Azure cold-tier)
- `synapsql-tenant`: ATTACH per-tenant isolation
- API-key auth (constant-time eq)
- Stripe metering integration
- LiveQuery WebSocket (Surreal-killer parity)

**Gate**: Turbopuffer-pricing-parity at $0 self-host. Multi-tenant per-row isolation.

## Mining strategy

`ghmax × 50` queries per phase, parallel worktree fanout, eval-harness bandit-merge.

## Bench targets after P5

| Workload | synapsql | Konkurrenz #1 | Vorsprung |
|----------|------------|---------------|-----------|
| TPC-C 8t | ≥1.5M QPS | SingleStore 1.5M | match |
| TPC-H 22q | <60s | DuckDB ~120s | 2× |
| ClickBench | top-3 | ClickHouse | 0.8-1.2× |
| Vec recall@10 (100M) | ≥0.99 @ 100k QPS | Pinecone | 5-10× |
| Hybrid (vec+FTS+SQL) | <5ms p50 | none does this | ∞ |

## Cost

- ghmax 50q/phase = 0€
- 12× Sonnet impl = ~5€
- 4× Opus arch = ~4€
- M4 Max bench = 0€
- **Total P0-P5: ~10€**
