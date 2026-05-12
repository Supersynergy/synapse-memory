# SynapsQL Feature Matrix — 2026-05-12

Source: `crates/synapsql/` + `crates/synapse-mysql/`
Backend: SynapsQL = thin wire-protocol shim → libSQL/SQLite store.
All standard SQL features = whatever the backend supports.

Legend: ✅ impl  🚧 partial/stub  ❌ missing/not wired

---

## A) Core SQL

| Feature | SynapsQL | MariaDB-11 | Postgres-17 | SingleStore | MyDuck | TiDB | SQLite |
|---------|----------|-----------|------------|-------------|--------|------|--------|
| CREATE/DROP/ALTER TABLE | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| PRIMARY / FOREIGN / UNIQUE KEY | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 FK optional |
| INSERT / UPDATE / DELETE / SELECT | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| WHERE / ORDER BY / GROUP BY / HAVING / LIMIT | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| JOIN (INNER/LEFT/RIGHT/FULL/CROSS) | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 no FULL |
| Subqueries (WHERE/FROM/scalar) | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| CTEs (WITH … AS) | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Recursive CTEs | ✅ passthrough | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ |
| Window functions | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| UNION / INTERSECT / EXCEPT | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| DISTINCT | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| CASE WHEN | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| NULL handling | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

**Note**: SynapsQL "passthrough" = shim forwards raw SQL to libSQL. No own SQL engine.

---

## B) Transactions

| Feature | SynapsQL | MariaDB-11 | Postgres-17 | SingleStore | MyDuck | TiDB | SQLite |
|---------|----------|-----------|------------|-------------|--------|------|--------|
| BEGIN / COMMIT / ROLLBACK | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| SAVEPOINT | ✅ passthrough | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ |
| Isolation levels | 🚧 SQLite only (SERIALIZABLE default) | ✅ 4 levels | ✅ 4 levels | ✅ RC only | ✅ | ✅ | ❌ only SERIALIZABLE |
| Deadlock detection | ❌ not impl | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |

---

## C) Indexes

| Feature | SynapsQL | MariaDB-11 | Postgres-17 | SingleStore | MyDuck | TiDB | SQLite |
|---------|----------|-----------|------------|-------------|--------|------|--------|
| B-tree | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| UNIQUE index | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Composite index | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Hash index | ❌ | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| FULLTEXT index | 🚧 FTS5 via SQLite (parse+intercept) | ✅ | ✅ GIN | ✅ | ❌ | ✅ | 🚧 FTS5 ext |
| Vector index (HNSW) ⭐ | 🚧 parse+detect only, execute=TODO stub | ❌ | ✅ pgvector | ✅ | ❌ | ❌ | ❌ |
| GIN/GiST | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ |

**Critical**: `VectorOp::execute()` returns `vec![]` — TODO comment, not wired to store.

---

## D) Data Types

| Type | SynapsQL | MariaDB-11 | Postgres-17 | SingleStore | MyDuck | TiDB | SQLite |
|------|----------|-----------|------------|-------------|--------|------|--------|
| INT/BIGINT/SMALLINT | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| FLOAT/DOUBLE/DECIMAL | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| VARCHAR/TEXT/BLOB | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| DATE/DATETIME/TIMESTAMP | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 text storage |
| JSON/JSONB | 🚧 passthrough, no JSON funcs impl | ✅ | ✅ JSONB | ✅ | ✅ | ✅ | ✅ |
| VECTOR/EMBEDDING ⭐ | 🚧 syntax detected, not stored natively | ❌ | ✅ pgvector | ✅ | ❌ | ❌ | ❌ |
| ENUM | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| UUID | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 text |
| Arrays | ❌ | ❌ | ✅ | ✅ | ✅ | ❌ | ❌ |

---

## E) Functions

| Function group | SynapsQL | MariaDB-11 | Postgres-17 | SingleStore | MyDuck | TiDB | SQLite |
|----------------|----------|-----------|------------|-------------|--------|------|--------|
| String (CONCAT, LEN, SUBSTR, REPLACE…) | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Math (ABS, ROUND, FLOOR, CEIL…) | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Date (NOW, DATE_ADD, DATEDIFF…) | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 partial |
| Aggregate (COUNT/SUM/AVG/MIN/MAX) | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| JSON (JSON_EXTRACT, JSON_SET…) | ❌ not intercepted | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ JSON1 |
| Window (ROW_NUMBER, RANK, LAG…) | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Vector (<=> / COSINE / DOT…) ⭐ | 🚧 `<=>` parse only, execute stub | ❌ | ✅ pgvector | ✅ | ❌ | ❌ | ❌ |
| FTS (MATCH/AGAINST) ⭐ | 🚧 parse + rewrite stub, no real exec | ✅ | ✅ tsvector | ✅ | ❌ | ✅ | 🚧 FTS5 ext |
| HYBRID_RANK ⭐ | 🚧 RRF math impl, full 3-arg wiring TODO | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

## F) Operations

| Feature | SynapsQL | MariaDB-11 | Postgres-17 | SingleStore | MyDuck | TiDB | SQLite |
|---------|----------|-----------|------------|-------------|--------|------|--------|
| EXPLAIN / EXPLAIN ANALYZE | 🚧 passthrough (no own plan) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Stored procedures | ❌ | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| Triggers | ❌ | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |
| Views | ✅ passthrough | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Materialized views | ❌ | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ |
| GRANT / REVOKE (user perms) | ❌ | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| TLS connections | 🚧 not wired in server | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| Replication | ❌ | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| Backup / Restore | ❌ | ✅ | ✅ | ✅ | ❌ | ✅ | 🚧 file copy |
| Point-in-time recovery | ❌ | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| Performance Schema | ❌ (only QPS counter) | ✅ | ✅ pg_stat | ✅ | ❌ | ✅ | ❌ |

---

## G) Synapse-Killer (UNIQUE)

| Feature | SynapsQL | MariaDB | Postgres | SingleStore | MyDuck | TiDB | SQLite |
|---------|----------|---------|---------|-------------|--------|------|--------|
| `<=>` vector op ⭐ | 🚧 parse ✅ execute ❌ | ❌ | ❌ native | ❌ | ❌ | ❌ | ❌ |
| HYBRID_RANK (RRF) ⭐ | 🚧 math ✅ wire ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| WITH RECALL_GUARANTEE ⭐ | 🚧 parse ✅ wire ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| AS OF time-travel ⭐ | 🚧 parse ✅ wire ❌ timestamp stub broken | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| WITH GRAPH_TRAVERSE ⭐ | ✅ full expand to recursive CTE | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

**Only GRAPH_TRAVERSE is fully implemented end-to-end.**

---

## Coverage Summary

| Category | Implemented | Partial | Missing | Coverage vs MariaDB |
|----------|------------|---------|---------|---------------------|
| Core SQL (13) | 13 | 0 | 0 | ~95% (passthrough) |
| Transactions (4) | 2 | 1 | 1 | ~50% |
| Indexes (7) | 3 | 2 | 2 | ~43% |
| Data Types (9) | 6 | 2 | 1 | ~67% |
| Functions (8) | 5 | 3 | 0 | ~63% |
| Operations (11) | 2 | 2 | 7 | ~27% |
| Synapse-Killer (5) | 1 | 4 | 0 | N/A (unique) |

**Overall vs MariaDB: ~55%** (core SQL counts but most production features missing)

---

## TOP-10 Missing Features — Ranked by (Impact × Frequency × Effort⁻¹)

| Rank | Feature | Why High | Effort | Score |
|------|---------|---------|--------|-------|
| 1 | **Wire `<=>` execute to synapse-core vec-search** | Killer feature, TODO stub — app unusable without | Low (1-2d) | 🔥🔥🔥🔥🔥 |
| 2 | **Wire HYBRID_RANK full 3-arg** | Core differentiator, RRF math done, just missing store bridge | Low (1-2d) | 🔥🔥🔥🔥🔥 |
| 3 | **Fix AS OF timestamp parse** (chrono stub broken — always returns now()) | Time-travel is useless if timestamp=now() | Low (hours) | 🔥🔥🔥🔥 |
| 4 | **Wire RECALL_GUARANTEE alpha → conformal_search** | Parse done, alpha silently discarded | Low (1d) | 🔥🔥🔥🔥 |
| 5 | **TLS for MySQL wire** | Production-blocker, any cloud deploy | Medium (2-3d) | 🔥🔥🔥🔥 |
| 6 | **Prepared-stmt param types** (on_prepare returns 0 params/cols) | ORMs break (SQLAlchemy, Prisma, GORM) | Medium (3-5d) | 🔥🔥🔥 |
| 7 | **Return typed columns** (all cols = MYSQL_TYPE_VAR_STRING) | BI tools, Grafana, Metabase break | Medium (3-5d) | 🔥🔥🔥 |
| 8 | **JSON functions** (JSON_EXTRACT etc) | 80% of modern apps use JSON | Medium (3-5d) | 🔥🔥🔥 |
| 9 | **SHOW TABLES real impl** (returns empty []) | `\dt`, DBeaver, MySQL Workbench broken | Low (1d) | 🔥🔥🔥 |
| 10 | **GRANT/REVOKE + auth** | Multi-tenant blocked, security gap | High (1-2w) | 🔥🔥 |

---

## Empfehlung Reihenfolge

**Sprint 1 (diese Woche, high ROI):**
1. Wire `VectorOp::execute` → `Store::vec_search` — killer feature, 1 TODO zeile
2. Wire HYBRID_RANK 3-arg — RRF math fertig, nur store-bridge fehlt
3. Fix AS OF timestamp: `chrono::DateTime::parse_from_rfc3339(ts_str)` statt now()
4. Wire RECALL_GUARANTEE → `SearchOptions.conformal_target`
5. SHOW TABLES → echte store table-list

**Sprint 2 (woche 2):**
6. Typed column metadata in MySQL wire (Column.coltype)
7. Prepared-stmt param count/types
8. TLS (rustls in tokio TCP accept)

**Sprint 3 (longer term):**
9. JSON function UDFs
10. GRANT/REVOKE basic auth

**Fazit**: SynapsQL hat exzellente Parser-Infrastruktur und unique Synapse-Extensions.
Aber alle Killer-Features sind "parse-only" — execute=TODO. Sprint 1 würde SynapsQL
von Prototype → Production bringen in ~3-4 Tagen.
