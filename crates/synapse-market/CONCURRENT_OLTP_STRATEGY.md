# Concurrent OLTP Strategy for synapse-market

## Honest Concurrency Limits

### SQLite WAL (default synapse-market backend)

SQLite WAL allows **1 writer at a time + unlimited concurrent readers**.
There is no multi-writer MVCC. All writes serialize.

- `PRAGMA journal_mode=WAL` → readers never block writers, writers never block readers.
- Multiple concurrent `INSERT`/`UPDATE` → serialized under a single WAL lock.
- **Not a MySQL/Postgres replacement for concurrent OLTP workloads.**

### libSQL async-WAL (synapse-libsql backend)

`libSQL` extends SQLite with `BEGIN CONCURRENT` — optimistic multi-writer transactions that only conflict on overlapping row ranges. This is the right backend if you need:

- Multiple independent writers (e.g. ingesting from several feeds simultaneously).
- Distributed edge deployments (Turso).

→ See `crates/synapse-libsql/` for the existing scaffold.

## Synapse-X / synapse-market Architecture

synapse-market uses an **append-only columnar log + COW snapshots** per Series:

```
Producer (1 thread) → append log → COW snapshot
Consumers (N)       → mmap read  → lockless
```

This is NOT a multi-writer design by intent. The access pattern is:

| Access type | Expected concurrency | Mechanism |
|-------------|---------------------|-----------|
| OHLCV ingest | 1 producer per ticker | WAL batch-insert |
| Range queries | N parallel readers | WAL read-only, mmap |
| Signal index | read-only after build | mmap, lockless |
| Correlation | N parallel reads | SQLite shared cache |

Single-producer + N-reader via mmap is **optimal for quant workloads** — this is the kdb+ model.

## We Are NOT a MySQL/Postgres Replacement

synapse-market is a **kdb+-class embedded quant engine**, not a general OLTP database:

| Workload | synapse-market | MySQL/Postgres |
|----------|---------------|----------------|
| OHLCV time-series range scan | ✅ native, SIMD | ❌ row-store, slow |
| Signal similarity search | ✅ RaBitQ / TurboVec | ❌ no native ANN |
| Correlation matrix | ✅ AMX/Accelerate | ❌ requires extension |
| Concurrent OLTP (e-commerce, CRM) | ❌ single writer | ✅ native MVCC |
| Multi-writer ETL pipelines | ❌ WAL bottleneck | ✅ row-level locking |

## Strategy: When to Use Which Backend

### Use synapse-market (SQLite WAL) when:
- Single-producer tick ingestion per symbol.
- Read-heavy: backtesting, analytics, signal replay.
- Embedded, zero-process, no network overhead.
- Quant research, strategy development, HFT simulation.

### Use synapse-libsql (BEGIN CONCURRENT) when:
- Multiple simultaneous writers (e.g. multi-feed live trading).
- Distributed deployment (Turso edge nodes).
- Need optimistic MVCC without switching away from SQLite semantics.

### Use Postgres/MySQL when:
- True OLTP: concurrent INSERT/UPDATE/DELETE from many clients.
- ACID multi-table transactions with row-level locking.
- Large web application backends (e-commerce, CRM, etc.).

## Position Statement

> synapse-market is a **kdb+-replacement for quant workflows**.
> It is not a Postgres or MySQL replacement.
> For quant: single-producer append + N-reader mmap = optimal.
> For concurrent OLTP: use `synapse-libsql` (EXISTS) or Postgres.

## References

- `crates/synapse-libsql/` — libSQL async-WAL backend with `BEGIN CONCURRENT` support.
- `crates/synapse-mysql/` — MySQL wire-protocol proxy over any `Store` impl.
- `src/bin/smx_mysql_shim.rs` — MySQL wire-protocol shim exposing `candles` and `corr_matrix` directly from synapse-market.
- synapse_truth 2026-05-10: "NOT faster MySQL concurrent OLTP due to msql_srv-blocking-sync + SQLite-WAL single-writer."
