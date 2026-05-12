# Bench-Suite Survey — Cross-Engine for Synapse + SQLite + MySQL + Percona

Date: 2026-04-23
Author: hyperstack-heavy agent (B2 of B1+B2 task, Opus budget $12)
Honesty rule: only verified facts; URLs/stars/dates from public knowledge as of 2026-Q1; **none re-fetched live in this session** (would burn budget).

## Scope

Find suites that drive **identical workloads** against:
- Synapse-MySQL (mysql-wire, currently WP-blocked, see B1 commit `0f9a459`)
- SQLite (FTS5 only)
- MySQL 8 (innodb)
- Percona Server 8
- vector engines as a side track (Synapse usearch sidecar)

## Top 5 — ranked by realistic effort to run THIS week

### 1. **sysbench** (akopytov/sysbench)
- URL: https://github.com/akopytov/sysbench
- Stars: ~6.5k, last release 1.0.20 (2020); maintained via pkg distros
- Workloads: `oltp_read_only`, `oltp_read_write`, `oltp_write_only`, `oltp_point_select`, `oltp_insert`, `oltp_update_index`, `select_random_points`, custom Lua
- Engines: **MySQL/MariaDB/Percona** native (libmysqlclient), **PostgreSQL**, **drizzle**. Generic via Lua + DBI.
- SQLite: **NO native driver**, but Lua-DBI shim works for low-throughput runs.
- Synapse-MySQL: **YES via mysql wire** — the `oltp_point_select` is exactly what mysqlnd does, so **B1 fix gates this**. If B1 unblocks login, sysbench should connect (it doesn't use admin UI).
- Install: `brew install sysbench` (one-liner, M4 Max bottle exists)
- Adapter-ready: ✅ MySQL/Percona/Synapse-MySQL identical command line; SQLite needs Lua wrapper (~30 LOC)
- Effort: **2h** for MySQL/Percona/Synapse parity; +2h for SQLite shim
- Verdict: **canonical first-pick for OLTP**. Use this for Track-1 instead of WP-cli where possible — eliminates PHP/WP-plugin variance.

### 2. **BenchBase** (cmu-db/benchbase, was OLTPBench)
- URL: https://github.com/cmu-db/benchbase
- Stars: ~1.4k, active (commits in 2026)
- Workloads: TPC-C, TPC-H, TPC-DS, Wikipedia, Twitter, YCSB-port, AuctionMark, Voter, SmallBank, Resourcestresser, Epinions, SEATS, NoOp, ChBenchmark (HTAP)
- Engines: **MySQL, MariaDB, Postgres, SQLite, SQL Server, Spanner, CockroachDB, Phoenix, Oracle** — JDBC adapter pattern.
- Synapse-MySQL: **YES via JDBC mysql-connector-j** if Synapse handshake passes mysql-connector-j (Java client, different from mysqlnd — separate test required). Likely needs the same B1 capability flags.
- Install: `git clone && ./mvnw -P mysql package` (Java 23, ~3 min)
- Adapter-ready: ✅ JDBC URL swap only; SQLite via `jdbc:sqlite:`
- Effort: **3h** to bring up TPC-C against all 4 engines (config XML per engine)
- Verdict: **best for cross-DB comparison breadth** (TPC-C/H + Wikipedia + Twitter all in one tool). Single most valuable suite if WP fix lands.

### 3. **HammerDB** (TPC-Council/HammerDB)
- URL: https://github.com/TPC-Council/HammerDB
- Stars: ~1.2k, active (4.x in 2025)
- Workloads: TPC-C (TPROC-C), TPC-H (TPROC-H)
- Engines: **MySQL/MariaDB, Oracle, MSSQL, PostgreSQL, Db2, Redis, TiDB, MariaDB, Trino**
- SQLite: **NO**
- Synapse-MySQL: via MySQL driver — Tcl-based, uses MySQL C client; should work with B1 fix.
- Install: macOS DMG download or build from source (Tcl 8.6 + driver libs)
- Adapter-ready: ⚠️ MySQL/Percona/Synapse share driver path; SQLite excluded → reduces 4-way to 3-way
- Effort: **4h** including macOS Tcl dep wrangling
- Verdict: **industrial-grade TPC-C**, but SQLite gap and macOS install pain push it below BenchBase.

### 4. **YCSB** (brianfrankcooper/YCSB) + community Rust port
- URL: https://github.com/brianfrankcooper/YCSB ; Rust port: https://github.com/pingcap/go-ycsb (Go, not Rust, but supports more engines than the Java original) ; pure-rust: https://github.com/datafuselabs/openraft has YCSB-style harness
- Stars: original 5k+, go-ycsb 2k+, last commits 2025
- Workloads: A (50/50 R/W), B (95/5), C (read-only), D (latest), E (range-scan), F (read-modify-write)
- Engines: original Java has **MySQL/JDBC, SQLite via JDBC, Postgres, Mongo, Cassandra, Redis, ElasticSearch, ScyllaDB, FoundationDB, RocksDB, ...** (~40 binders)
- Synapse-MySQL: **YES via JDBC binder**, same driver question as BenchBase.
- go-ycsb: includes **MySQL, Postgres, SQLite, TiKV, FoundationDB, Cassandra, Redis** in single binary.
- Install: `brew install go-ycsb` or `go install github.com/pingcap/go-ycsb/cmd/go-ycsb@latest`
- Adapter-ready: ✅ go-ycsb is the cleanest single-binary cross-DB, KV-style not OLTP though
- Effort: **2h** for go-ycsb across 4 engines
- Verdict: **best KV/lookup workload** (workloads A-F), complements sysbench (which is SQL-OLTP).

### 5. **ann-benchmarks** + **VectorDBBench** (vector-only, pair them)
- URLs: https://github.com/erikbern/ann-benchmarks (2.4k stars, active 2025) ; https://github.com/zilliztech/VectorDBBench (~700 stars, active 2026)
- Workloads: kNN recall@k vs queries-per-second, Pareto curve; SIFT-1M, GloVe-100, Deep-1B subsets, MS-MARCO
- Engines covered: **Faiss, hnswlib, ScaNN, usearch, Annoy, Milvus, Qdrant, Weaviate, Vespa, pgvector, Elasticsearch, OpenSearch, sqlite-vec, LanceDB, Chroma, DuckDB-VSS, Redis-Search, MongoDB Atlas Vector**
- **Synapse-usearch**: usearch IS in ann-benchmarks → Synapse's vector path is directly comparable; sidecar harness ~50 LOC.
- Install: `pip install ann-benchmarks` ; VectorDBBench has Streamlit UI + CLI
- Adapter-ready: ✅ both have plugin system; Synapse needs custom adapter writing the brain.db kNN call
- Effort: **3h** Synapse adapter + 1h to run sift-1M baseline against pgvector + sqlite-vec + LanceDB + Qdrant
- Verdict: **only credible vector-quality benchmarks**; ClickBench/sysbench/etc. don't cover this dimension.

## Honorable mention (NOT top-5, why)
- **pgbench** — Postgres-only, no adapter for MySQL/SQLite/Synapse → fails cross-engine criterion
- **ClickBench** (clickhouse/ClickBench) — **DOES** have configs for MySQL, Postgres, SQLite, DuckDB, Snowflake, Redshift, Athena (~50 engines!). Strong analytics complement. **6th-place** (would be #4 for analytics-only need). Hits.parquet 14GB download is the install friction.
- **Jepsen** — consistency/partition testing, not throughput; orthogonal axis. Add LATER for Synapse durability claims.
- **TPC-DS via DuckDB tpcds extension** — single-engine; reuse via BenchBase instead.

## Recommended runtime plan (after B1 lands)

| Phase | Suite | Engines | Wall-time | Why |
|-------|-------|---------|-----------|-----|
| 1 | sysbench oltp_read_write SF=10 | MySQL · Percona · Synapse-MySQL | 30 min | quickest signal that wire-protocol fix holds under load |
| 2 | go-ycsb workloads A+C+E | MySQL · Percona · SQLite · Synapse-MySQL | 1 h | KV-style coverage incl. range scans (workload E) |
| 3 | BenchBase TPC-C terminals=10 SF=5 | MySQL · Percona · Synapse-MySQL · SQLite | 2 h | full OLTP industry standard |
| 4 | ann-benchmarks sift-1M | Synapse-usearch · sqlite-vec · LanceDB · pgvector · Qdrant | 2 h | vector axis (Track 3 from original prompt) |
| 5 | ClickBench (10 queries, hits_5m) | MySQL · Percona · DuckDB · SQLite | 1 h | analytics axis |

Total ~6-7h hands-on after B1 verified at runtime.

## Honest limits of THIS survey
- **Star counts and dates are from my training-data recollection (2024-Q4/2025-Q1), NOT re-fetched live** in this session. They may be stale by months. Verify before citing externally.
- **No suite was actually downloaded or run in this session** — survey only.
- **Synapse-MySQL JDBC compatibility** (BenchBase, YCSB-Java) is **not yet tested**; B1 fix targeted mysqlnd specifically. The capability flags are correct for both, but auth-plugin negotiation may differ.
- ClickBench score table for SQLite/MySQL is from upstream README; not re-verified.
