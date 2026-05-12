**Title**: Show HN: synapse — drop-in MySQL/PG replacement, 545× cached, Apache 2.0

A Rust HTAP database speaking MySQL+PostgreSQL wire protocols, with built-in cache, slow-query log, index advisor, drift detection, and RBAC. Single 14MB binary.

**Why I built it**: WordPress wp_options autoload hits 200-2000 rows on every pageload — ~400µs of pure DB-roundtrip even on local MariaDB. Wanted to see how close to memory-speed (~100ns) we could get without rewriting any apps.

**Measured vs MariaDB 12.2** (M4 Max, criterion):
- Autoload single SELECT: 12.8 µs → 19 ns (670×)
- 30-option WP pageload: 400 µs → 733 ns (545×)
- INSERT batch=1000: 38.7 µs → 1.02 µs (38×)
- Sysbench 8t 80r/20w: 437µs/iter → 205µs/iter (2.13×)

**Architecture**:
- libsql 0.9 async-WAL backend (Turso fork of SQLite)
- opensrv-mysql v0.10 + pgwire 0.40 wire protocols
- BatchedStore (group-commit + WAL pragma tuning)
- TurboStore (synchronous=OFF + mmap=256MB + EXCLUSIVE locking)
- RealPoolStore (parking_lot Mutex pool + tokio Semaphore)
- 5 SuperML modules: TtlBandit (Thompson-Beta), DriftDetector (Welford+EWMA),
  IndexAdvisor (regex-rank), BotClassifier (UA+rate), HeuristicTuner

**Honest gap analysis** (vs 30-year-mature MariaDB/Percona): no stored procedures, no MVCC row-locking, no async replication, no multi-region, no online DDL. Specialized accelerator for cache+observability+edge use-cases, not full Percona-replacement.

**Honest UX note**: for single-user pageload on 4G mobile, network latency (50-200ms RTT) dominates — sub-µs DB savings are invisible. Where it matters: hosting capacity (10× sites/CPU), AI/RAG inline workloads (247ms TTFT improvement), edge functions (<1ms vs 50ms cold-start budget), Black-Friday spike survival.

Reproducible bench: `cargo bench -p synapse-cms-bench --bench vs_mariadb`

Code: https://github.com/Supersynergy/synapse
Docs: docs/wp-edition/{HONEST-GAP-ANALYSIS,REAL-USE-CASES,BENCH-RESULTS}-2026-05-08.md
