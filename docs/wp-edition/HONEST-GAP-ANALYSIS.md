# synapsql vs MariaDB/Percona/MySQL/Postgres — Honest Gap Analysis

User-frage: *"was können andere DB was meine noch nicht kann auch in den ganzen sonderfällen?"*

## TL;DR
**synapsql ist heute (P1) kein production-replacement für MariaDB/Percona/MySQL/Postgres.** Wir gewinnen ausgewählte Workloads (cache, batch INSERT, mixed 80r/20w 8t) — aber die Konkurrenz hat 30+ Jahre engineering hinter sich. Diese Liste ist vollständig + ehrlich.

## 1. SQL Sprachfeatures (synapsql ❌, MySQL/PG ✅)

| Feature | MariaDB | Percona | MySQL 8 | PG 17 | synapsql | Notiz |
|---------|:-------:|:-------:|:-------:|:-----:|:--------:|-------|
| **JOINs** (INNER/LEFT/RIGHT/FULL/CROSS) | ✅ | ✅ | ✅ | ✅ | 🟡 libsql kann | aber kein optimizer |
| **Window functions** (ROW_NUMBER, LAG, RANK) | ✅ | ✅ | ✅ | ✅ | 🟡 libsql native | nicht beworben |
| **CTEs** (WITH ... AS, recursive) | ✅ | ✅ | ✅ | ✅ | 🟡 libsql native | |
| **JSON_EXTRACT/JSON_TABLE** | ✅ | ✅ | ✅ | ✅ JSONB | 🟡 SQLite json1 | weniger ergonomisch |
| **GIS / spatial** (POINT, ST_DISTANCE, R-tree) | ✅ | ✅ | ✅ | ✅ PostGIS | 🟡 SQLite R-tree | sehr begrenzt |
| **Stored procedures** | ✅ PL/SQL | ✅ | ✅ | ✅ PL/pgSQL | ❌ | nicht in libsql |
| **Triggers** (BEFORE/AFTER) | ✅ | ✅ | ✅ | ✅ | 🟡 SQLite triggers | |
| **Views** + Materialized views | ✅ MV | ✅ | ✅ | ✅ MV | 🟡 views nur | keine MV |
| **Foreign keys** + ON DELETE CASCADE | ✅ | ✅ | ✅ | ✅ | ✅ libsql | |
| **CHECK constraints** | ✅ | ✅ | ✅ | ✅ | ✅ libsql | |
| **Generated columns** | ✅ STORED+VIRTUAL | ✅ | ✅ | ✅ | ✅ libsql | |
| **Full-text** (MATCH/AGAINST, GIN tsvector) | ✅ FULLTEXT | ✅ | ✅ | ✅ tsvector | 🟡 FTS5 | extension |
| **Sequences** (NEXTVAL) | ✅ | ✅ | ✅ | ✅ | ❌ | nur AUTOINCREMENT |
| **EXPLAIN ANALYZE** plan | ✅ | ✅ ProfileTools | ✅ | ✅ | 🟡 SQLite EXPLAIN | weniger Detail |
| **Cost-based optimizer** | ✅ | ✅ | ✅ | ✅ | ❌ | SQLite has rule-based |
| **Query rewrite hints** | ✅ STRAIGHT_JOIN | ✅ | ✅ | ✅ | ❌ | |
| **Row-level locking** | ✅ InnoDB | ✅ XtraDB | ✅ | ✅ MVCC | ❌ table-level WAL | SQLite single-writer |
| **Multi-statement transactions** | ✅ | ✅ | ✅ | ✅ | ✅ libsql | |
| **Savepoints** | ✅ | ✅ | ✅ | ✅ | ✅ libsql | |

## 2. OLTP/OLAP fehlende Funktionen

| Feature | Konkurrenz | synapsql |
|---------|------------|---------|
| **MVCC** (multi-version concurrency, readers don't block writers) | ✅ InnoDB/PG | ❌ WAL = 1 writer |
| **Row-level locks** | ✅ | ❌ table/page |
| **Hot backup** (mysqldump live, pg_dump live) | ✅ | 🟡 sqlite .backup file copy |
| **Point-in-time recovery (PITR)** | ✅ binlog | ❌ |
| **Streaming replication** (master→slave async) | ✅ | ❌ (P3 raft planned) |
| **Group replication** (Galera, GR) | ✅ Galera | ❌ |
| **Read replicas** | ✅ | ❌ |
| **Partitioning** (RANGE, LIST, HASH) | ✅ | ❌ |
| **Sharding** (Vitess, Citus) | ✅ external | ❌ |
| **Online schema change** (gh-ost, pt-osc) | ✅ | ❌ |
| **Parallel query execution** | ✅ | ❌ single-threaded SQLite |
| **Buffer pool** mit eviction tuning | ✅ InnoDB BP | 🟡 mmap+page-cache |
| **Query cache** (deprecated MySQL aber some) | ✅ | 🟡 our AutoloadCache layer |
| **Hash join, merge join, nested-loop** | ✅ alle 3 | 🟡 nested-loop only |
| **Index merge** (intersect/union mehrerer indexes) | ✅ | ❌ |
| **Covering indexes** | ✅ | ✅ libsql |
| **Function indexes** | ✅ | ✅ libsql |
| **Partial indexes** | 🟡 PG ✅ | ✅ libsql |
| **Bitmap indexes** | 🟡 | ❌ |

## 3. Concurrency / Scale

| | Konkurrenz | synapsql |
|---|-----------|---------|
| **Concurrent writers** | ✅ row-level | ❌ 1 writer at a time (WAL) |
| **Reader-writer parallelism** | ✅ MVCC | ✅ WAL readers don't block |
| **Connection pooling built-in** | 🟡 ProxySQL | ✅ RealPoolStore |
| **Query timeout / kill** | ✅ KILL QUERY | 🟡 busy_timeout |
| **Resource governor** (per-user/db quotas) | ✅ MariaDB | ❌ |
| **Multi-tenant isolation** | ✅ user privileges | 🟡 ATTACH (planned P5) |
| **Workload manager** | ✅ MaxScale Workload | ❌ |
| **Online DDL** | ✅ (mostly) | ❌ table locks |
| **Foreign data wrappers** (postgres_fdw, FEDERATED) | ✅ | ❌ |

## 4. Auth / Security

| | Konkurrenz | synapsql |
|---|-----------|---------|
| **User accounts** with GRANT/REVOKE | ✅ | 🟡 P1.5 — synapsql-auth: SHA256+constant-time API-keys + RBAC (ReadOnly/ReadWrite/Admin) |
| **Roles** | ✅ | ❌ |
| **Row-level security (RLS)** | ✅ PG RLS | ❌ |
| **TLS connections** | ✅ | ❌ (mysql/pg wire ohne TLS) |
| **SHA256/caching_sha2 auth** | ✅ | ❌ |
| **Audit log** | ✅ MariaDB plugin | ❌ |
| **Encryption at rest** (transparent) | ✅ TDE | ❌ (file-system level only) |
| **Encrypted connections enforcement** | ✅ require_ssl | ❌ |
| **Account lockout / failed login** | ✅ | ❌ |
| **Password rotation / expiry** | ✅ | ❌ |

## 5. Replication / HA

| | Konkurrenz | synapsql |
|---|-----------|---------|
| **Async master-slave** | ✅ | ❌ |
| **Semi-sync replication** | ✅ | ❌ |
| **Sync replication** (Galera, PXC) | ✅ | ❌ |
| **Logical replication** (pg_logical) | ✅ PG | ❌ |
| **Failover automation** | ✅ Orchestrator/MHA | ❌ |
| **GTID-based replication** | ✅ | ❌ |
| **Multi-source replication** | ✅ MariaDB | ❌ |
| **Read scaling via replicas** | ✅ | ❌ |
| **Backup tools** (xtrabackup, percona xtrabackup) | ✅ | ❌ |
| **Incremental backup** | ✅ XtraBackup 2-3× faster (2026 release) | ❌ |

## 6. Observability

| | Konkurrenz | synapsql |
|---|-----------|---------|
| **performance_schema** (per-query stats) | ✅ MySQL/Maria | ❌ |
| **slow query log** | ✅ | 🟡 P1.5 — synapsql-ops::SlowQueryLog: threshold-based, top-N, JSON serializable |
| **EXPLAIN FORMAT=JSON/TREE/ANALYZE** | ✅ | 🟡 sqlite explain |
| **Query store / query digest** | ✅ MariaDB | ❌ |
| **InnoDB metrics** (BP hit ratio, locks, latches) | ✅ | ❌ |
| **Prometheus exporter** | ✅ mysqld_exporter | ❌ |
| **Top-N query analyzer** (pt-query-digest) | ✅ Percona toolkit | ❌ |
| **Lock wait analysis** | ✅ INFORMATION_SCHEMA | ❌ |
| **Histogram statistics** | ✅ | ❌ |

## 7. Tooling Ecosystem

| | Konkurrenz | synapsql |
|---|-----------|---------|
| **mysqldump / pg_dump** | ✅ | 🟡 P1.5 — synapsql-ops::Backup snapshot to local file (S3 P5) |
| **Migration tools** (flyway, gh-ost, liquibase) | ✅ | 🟡 generic SQL works |
| **GUI clients** (Workbench, DBeaver, TablePlus, pgAdmin) | ✅ via wire | ✅ via mysql/pg wire |
| **ORM compat** (Drizzle, Prisma, Diesel, ActiveRecord) | ✅ | 🟡 if SQL is generic enough |
| **Cloud-managed** (RDS, Aurora, Cloud SQL, Atlas-equivalent) | ✅ | ❌ |
| **Kubernetes operators** | ✅ | ❌ |
| **WP plugin marketplace** | ✅ | ❌ (synapsql wp-rocket replace planned P5) |
| **Stored libraries** (Boost.SQL, libpqxx) | ✅ | 🟡 mysql/pg crate works via wire |

## 8. Edge cases synapsql wird scheitern

1. **High write contention OLTP** (Black Friday checkouts) — single writer WAL bottleneck. RealPoolStore semaphored aber WAL serialisiert echte writes.
2. **Long-running analytics** (TPC-H 22q) — kein parallel execution, kein columnar (P2 plan).
3. **Multi-table joins on big tables** — kein cost-optimizer, libsql nested-loop only.
4. **Stored procedures** für legacy apps — komplett missing.
5. **Trigger-heavy schemas** — libsql triggers funktionieren aber langsam.
6. **GIS queries** (PostGIS users) — basic R-tree only.
7. **Per-user permissions** — kein auth-system. Wire ist offen.
8. **TLS** — Wire-Server akzeptiert plain. Production = MITM-Risiko.
9. **PITR / Backup recovery** — kein binlog. Verlorene Daten = verloren.
10. **Replication für read scaling** — kein async-replication.
11. **Schema migrations on big tables** — kein gh-ost-pattern.
12. **Online ALTER TABLE** — locks the table.

## Wann **nicht** synapsql nehmen (P1 today)
- Production e-com mit echtem Geld-flow ohne backup-strategy
- Apps mit hohen Compliance-Anforderungen (SOC2, GDPR audit log)
- Multi-region distributed systems
- High-write contention (>10k concurrent writers)
- Apps die stored procedures nutzen
- Legacy MySQL with complex GRANTs

## Wann synapsql JETZT hervorragend ist
- ✅ Read-heavy WordPress/Ghost/Strapi sites (cache wins big)
- ✅ Embedded apps (single binary deploy)
- ✅ Dev/test envs (fast spinup, deterministic)
- ✅ Edge functions (cold start <100ms)
- ✅ Multi-tenant SaaS where each tenant = own .db (planned P5)
- ✅ Vec+FTS+Graph workloads (no other DB has all 3 native)

## Roadmap to close gaps

| Gap | Phase | ETA |
|-----|-------|-----|
| MVCC / row-locking | P5 | 12w (libsql HCTREE branch) |
| Async replication | P3 | 4w (openraft) |
| Stored procedures | P8 | 16w (lua/wasm runtime) |
| TLS wire | P2 | 1w (rustls + opensrv-mysql tls feature) |
| User auth + GRANT | P3 | 4w (mining: MySQL grant tables) |
| Backup tooling (xtrabackup-style) | P4 | 6w |
| Cost-based optimizer | P6 | 12w (DataFusion adapter) |
| Columnar OLAP path | P2 | 2w (Lance format) |
| Stored procedures | P8 | 16w |

## Verdict

**Heute**: synapsql is a **specialized accelerator** für ausgewählte WP/cache/vec workloads. NICHT als full-replacement für MariaDB/Percona/MySQL.

**Year-1 Plan**: schrittweise gaps schließen via mining + ghmax. Nach 12 Monaten realistisch ein production-fähiger Konkurrent für 80% MySQL workloads. Niemals 100% — manche Konkurrenz-Features (MVCC, parallel exec, stored procs) brauchen massive engineering.

**Was Konkurrenz NIE einholen wird** (synapsql USPs):
- Library + Daemon + Wire-Proxy gleichzeitig
- 4 Wires gleichzeitig
- 1-bit Hamming compress (71×)
- AI-native (vec+FTS+graph+rerank in single SQL)
- M-series Metal/MLX kernels
- Single binary deploy
- Apache 2.0 EU self-host
- Platform-aware optimizer (WP/Woo/Shopify/...)
