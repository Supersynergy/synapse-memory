# Synapse vs MySQL — Fundamental Limits

## What Synapse-MySQL IS

A MySQL wire-protocol proxy that translates MySQL queries to SQLite. Designed for:
- Embedded deployments where MySQL is too heavy
- Single-node read-heavy workloads with <4 concurrent clients
- WordPress/CMS at low traffic (< 50 concurrent users)
- Development/test environments needing a MySQL-compatible SQLite backend
- Offline-first apps with sync-on-demand

## What Synapse-MySQL is NOT

Not a MySQL replacement for:
- High-concurrency OLTP (>4 threads, >1000 OPS sustained)
- Write-heavy workloads (INSERTs/UPDATEs > 500/s sustained)
- Latency-sensitive applications requiring < 2ms avg response
- Multi-tenant SaaS with isolation requirements

## Benchmark Reality (2026-04-24, M4 Max)

| Workload | Synapse v5 | MySQL 8 | Gap |
|----------|------------|---------|-----|
| C (100% reads, 8t) | ~700-944 OPS | 63,402 OPS | 67-90× |
| B (95% reads, 8t) | ~650 OPS | 27,091 OPS | 41× |
| A (50r/50w, 8t) | ~517 OPS | 5,520 OPS | 10× |
| F (RMW, 8t) | ~526 OPS | 12,333 OPS | 23× |
| Single-thread read | ~2,000+ OPS | ~8,000 OPS | 4× |

## Structural Bottlenecks (ordered by impact)

### 1. Blocking MySQL Protocol (msql_srv crate)
**Impact**: 70% of gap

`msql_srv::MysqlIntermediary::run_on_tcp` is a synchronous blocking loop — one OS thread per
connection. The crate uses `io::Read + io::Write` traits, not `AsyncRead + AsyncWrite`. Async
cannot be layered on top without replacing the entire protocol implementation.

**What it would take to fix**: Rewrite MySQL wire protocol handler using `tokio` + async traits.
Open source alternatives: `opensrv-mysql` (async), but requires full port of all `MysqlShim`
callbacks. Estimated effort: 2-3 weeks.

### 2. SQLite WAL vs InnoDB MVCC
**Impact**: 20% of gap on read-heavy, 50% on write workloads

SQLite WAL mode allows multiple concurrent readers + one writer. But:
- All readers share the same WAL file → page cache contention under 8+ readers
- Writers block all readers during checkpoint
- No row-level locking → table-level isolation only
- InnoDB: per-row versioning, readers never block writers, writers never block readers

**What it would take to fix**: SQLite 3.37+ with `BEGIN CONCURRENT` (TigerBeetle fork). Still
experimental. Alternatively: switch backend to libSQL (Turso) which has async WAL mode. Estimated
effort: 1 week for libSQL migration.

### 3. Result Cache TTL Invalidation
**Impact**: 10% of gap for mixed workloads

Current cache: LRU 4096 entries, 500ms TTL, no write-epoch invalidation. Under write workloads,
cache is correct but stale. The global write epoch is tracked but not used to invalidate entries.

**What it would take to fix**: Enable epoch-based invalidation. Simple 1-line fix but risks
correctness if epoch races. Per-table epoch tracking would be more correct.

### 4. Connection Setup Overhead
**Impact**: Minor (<5%)

Each MySQL connection opens a new `rusqlite::Connection` with full pragma initialization
(~8 pragmas, ~2ms). Under high connection churn this adds up. A pre-warmed connection pool
would help, but attempts showed mutex contention > startup savings at 8 threads.

## Where Synapse-MySQL Wins

| Scenario | Synapse | MySQL |
|----------|---------|-------|
| Disk space | 1 file, <1MB | 500MB+ data dir |
| Memory | 64MB | 512MB+ buffer pool |
| Cold start | <100ms | 5-30s |
| Backup | `cp file.db` | mysqldump / xtrabackup |
| Embedded (no daemon) | Library mode, 6µs reads | Not possible |
| Zero config | Just a binary | my.cnf, grants, init |
| Single-user apps | Full MySQL compat | Overkill |

## Library Mode (the real Synapse)

The MySQL proxy is a compatibility shim. The real performance is library mode:
- Vec search: 6µs (sqlite-vec kNN)
- Document put: 94µs
- FTS5 search: 0.4ms on 11k docs

These numbers are 100-1000× faster than any MySQL proxy can achieve because there's no
protocol overhead, no thread scheduling, no TCP.

**Rule**: Use library mode (`synapse_core::Store`) for new applications. Use MySQL proxy
only when you're migrating an existing MySQL-dependent app to SQLite.

## Roadmap to Close the Gap (if needed)

1. **Easy (1 day)**: Enable epoch-based cache invalidation, increase LRU to 16k entries
2. **Medium (1 week)**: Port to `opensrv-mysql` async protocol → removes thread-per-conn
3. **Hard (2-3 weeks)**: Switch rusqlite to libSQL async + WAL concurrent mode
4. **Architecture change**: Use DuckDB as MySQL backend for analytics workloads (10-100× on reads)
