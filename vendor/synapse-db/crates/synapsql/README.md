# SynapsQL

**MySQL/Postgres wire-compatible SQL server with native vector + FTS + graph in one Rust binary.**

## 5 Killer Numbers

| Metric | Target | Pattern |
|--------|--------|---------|
| **100k+ QPS** single binary | Multi-core tokio-task-per-conn | DragonflyDB shared-nothing |
| **≤50µs p99** local latency | Zero-copy Bytes result streaming | opensrv-mysql async wire |
| **1000 stmts/conn** cache | LRU per-connection stmt cache | Vitess vtgate |
| **HNSW rewrite** `<=>` | `WHERE embedding <=> :q` → ANN search | MyDuck extension router |
| **RRF hybrid** | `HYBRID_RANK(body, emb, :q)` BM25+vec | Synapse MUVERA fusion |

## Start

```bash
cargo build -p synapsql --release
./target/release/synapsql start --mysql 127.0.0.1:3306

# Smoke test
mysql -h 127.0.0.1 -P 3306 -e "SELECT 1"
mysql -h 127.0.0.1 -P 3306 -e "SELECT VERSION()"
mysql -h 127.0.0.1 -P 3306 -e "SHOW VARIABLES LIKE 'version'"
```

## SQL Extensions

```sql
-- Vector ANN search (HNSW rewrite)
SELECT id, body FROM docs WHERE embedding <=> :q LIMIT 10;

-- With conformal recall guarantee (SynapsQL exclusive)
SELECT id FROM docs WHERE embedding <=> :q LIMIT 10 WITH RECALL_GUARANTEE 0.99;

-- Full-text BM25
SELECT id FROM docs WHERE MATCH(body) AGAINST (:q);

-- Hybrid RRF (BM25 + vec, industry-best)
SELECT id, HYBRID_RANK(body, embedding, :q) AS score FROM docs ORDER BY score DESC LIMIT 20;
```

## Architecture

```
client → opensrv-mysql wire
           └→ ShimAdapter (per-conn)
               ├→ fingerprint (ProxySQL pattern)
               ├→ PreparedStmtCache (Vitess pattern, 1000/conn LRU)
               ├→ introspection intercept (SELECT 1, VERSION, SHOW)
               ├→ QueryCache (blake3-keyed LRU, write-epoch invalidation)
               ├→ rewriter (vec/<=> → HNSW, MATCH → BM25, HYBRID_RANK → RRF)
               └→ Store (synapse-core Brain — pluggable backend)
```

## Patterns Stolen From

- **opensrv-mysql** — async MySQL wire protocol foundation
- **ProxySQL** — statement fingerprinting (literal normalization → `?`)
- **Vitess vtgate** — per-connection statement plan cache, query routing
- **MyDuck** — MySQL→DuckDB bridge, read/write split, SQL extension routing
- **DragonflyDB** — thread-per-core, shared-nothing model (via tokio task-per-conn)
- **TiDB** — range-partition hints (TODO: cluster mode)
- **Synapse MUVERA** — RRF hybrid fusion for HYBRID_RANK

## TODO (SHOULD/NICE wave)

- [ ] Wire synapse-core::Brain as real Store backend
- [ ] QueryCache integration in ShimAdapter hot path
- [ ] mysql_async-based QPS bench (true COM_QUERY throughput)
- [ ] pgwire read path: SELECT/COPY OUT streaming
- [ ] Binlog replication stub (HA via Vitess-style binlog events)
- [ ] TiDB-style range sharding for cluster mode
- [ ] mmap result sets >1MB (zero-copy large scans)
