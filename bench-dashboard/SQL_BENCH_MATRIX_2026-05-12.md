# SQL Benchmark Matrix 2026-05-12 (v2 — BrainAdapter MAX)

Platform: MacBook Pro M4 Max · 128GB RAM · macOS 15
Databases: MariaDB 11 (Docker :13307) · SQLite (embedded) · DuckDB 1.x (embedded) · SynapsQL (MySQL-wire :13308)
Dataset: 10k posts + 30k postmeta + 50 cats · 3 runs per category · median reported

> **v2 changes**: BrainAdapter MAX — persistent pool, FTS5 routing, real table seeding via MySQL-wire.
> All SynapsQL numbers now use real table queries via MySQL-wire (not CLI subprocesses).
> Bugfixes: http::serve pending-fix (was crashing server), SET NAMES interception (was killing connections).

---

## Bench Matrix (v2 — real table queries)

| # | Kategorie | MariaDB | SQLite | DuckDB | SynapsQL | Winner |
|---|-----------|---------|--------|--------|----------|--------|
| 1 | **OLTP point** (100 indexed lookups) | 24.8ms | **0.40ms** | 8.3ms | 6.5ms | SQLite **62×** vs MariaDB |
| 2 | **OLTP write** (1k batch INSERT) | 7.9ms | **1.7ms** | 636ms | **1.8ms** | SQLite/SynapsQL tie |
| 3 | **OLAP aggregation** (GROUP BY 50 cats) | 1.5ms | 1.9ms | **0.25ms** | n/a | DuckDB **6×** vs MariaDB |
| 4 | **JOIN** (3-table, indexed) | 0.9ms | **0.42ms** | 0.79ms | n/a | SQLite **2.1×** vs MariaDB |
| 5 | **FTS search** (FTS5/MATCH/LIKE) | 6.6ms | **0.033ms** | 0.36ms | 0.26ms | SQLite **200×** vs MariaDB |
| 6 | **Vector search** (kNN proxy) | 1.7ms | 0.81ms | **0.90ms** | 10.9ms | DuckDB **1.9×** vs MariaDB |
| 7 | **Hybrid** (FTS + vec/score) | 11.1ms | 3.4ms | **0.99ms** | 10.5ms | DuckDB **11.2×** vs MariaDB |
| 8 | **Concurrent** (50 threads × 10 q) | 113ms | 36.3ms | 43ms | **35.0ms** | **SynapsQL** #1 |
| 9 | **Recovery** (cold connect + query) | 4.9ms | **0.11ms** | 4.7ms | 0.26ms | SQLite **45×** vs MariaDB |
| 10 | **ACID isolation** | PASS | PASS | PASS | PARTIAL | MariaDB/SQLite/DuckDB tie |
| 11 | **Storage** (10k posts + 30k meta) | 5968KB | 4492KB | **4364KB** | n/a | DuckDB **1.37×** smaller |
| 12 | **Setup time** (zero to query) | 346ms | **0.9ms** | 6.4ms | ~20ms | SQLite instant |

---

## Before/After Comparison (BrainAdapter MAX)

| Kategorie | Before (v1) | After (v2) | Improvement |
|-----------|-------------|------------|-------------|
| OLTP point | 79× behind SQLite* | 16.3× behind SQLite | 5× better |
| OLTP write | 5.4× behind SQLite* | **1.1× behind SQLite** | **5× better ✅ Target met** |
| FTS | 226× behind SQLite* | 7.8× behind SQLite | **29× better** |
| Concurrent | 8.7ms #1 | 35ms (parity SQLite) | Note: now real queries |
| Vec kNN | n/a / ERR | 10.9ms (wire-native) | Now functional |

*Previous v1 numbers: OLTP=`SELECT expr` no tables, FTS=CLI subprocess 10ms.

---

## Notes

**OLTP point** (16.3×): MySQL wire TCP loopback ~100µs/roundtrip vs SQLite embedded 4µs. Structural.
Target ≤2× requires embedded driver (no wire protocol). Not achievable via wire.

**OLTP write** (1.1×): Multi-row `INSERT VALUES (a),(b),...` eliminates TCP roundtrips. Matches SQLite. ✅

**FTS** (7.8×): FTS5 MATCH via MySQL-wire, 250µs vs SQLite 33µs direct. Previous 226× was subprocess.

**Vec kNN** (10.9ms): Wire-native `<=>` routing via BrainAdapter, real brain.db 113k docs. Was n/a.

**Concurrent** (35ms): Pool delivers 0-error 50-thread concurrency. SynapsQL #1 vs MariaDB (113ms).

**ACID**: WAL-mode cross-connection visibility works. No SQL-level `BEGIN/COMMIT` yet (PARTIAL).
