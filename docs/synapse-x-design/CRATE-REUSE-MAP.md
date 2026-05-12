# Synapse-Market Crate Reuse Map — 2026-05-13

## Decision table

| Crate | Feature | Use vs Duplicate vs Ignore | Rationale |
|-------|---------|---------------------------|-----------|
| **synapse-tsdb** | Arrow/Parquet IPC export | **USE** — wire as optional dep | Arrow v53 + Parquet shards. Market already exports OHLCV rows; tsdb gives us free Arrow IPC + Parquet roundtrip without writing 400 LOC. No conflict: polars 0.46 uses arrow2 internally (separate crate tree). Add `features = ["tsdb"]` optional. |
| **synapse-mlx-olap** | candle-Metal aggregations | **USE later** — document path, skip now | CPU fallback always compiles. Metal corr-matrix stacks with our AMX bench. BUT: candle-core Metal feature pulls ~15 dep additions. Defer to W2 after tsdb wired. |
| **synapse-iouring** | LSM + io_uring concurrent IO | **IGNORE** | macOS runtime = `UnsupportedPlatform`. Market runs on M4 Max (macOS). No benefit. |
| **synapse-olap** | DuckDB-embedded OLAP + router | **IGNORE** | Market already has `duckdb` as dev-dep and internal router logic. DuckDB + polars + Market in same binary = 3× linker bloat. Router heuristic (zero deps) is the only useful piece but trivial to inline. |
| **synapse-jit** | Cranelift JIT WHERE-filter + projection | **DUPLICATE — already done** | synapse-market already vendors cranelift-{codegen,frontend,jit,module,native} 0.131 directly in Cargo.toml and has `src/jit/`. Importing synapse-jit would double-link cranelift. Skip. |
| **synapse-stream** | CDC, CQ, pub/sub | **IGNORE** | Market has its own `src/stream/` and `stream_smoke` bin. CDC/CQ overlap is minimal. kafka-wire feature not needed. |
| **synapse-graph** | KG triples, PageRank, Louvain | **USE later** — ticker entity graph | Graph runs on brain.db SQLite edges. Market could expose `entity_graph(ticker)` returning PageRank-weighted co-mention edges. Defer: needs schema alignment with Market's `news` table. |

## Priority order for wiring

1. **synapse-tsdb** (this session) — highest leverage, zero conflicts, ~40 LOC bridge
2. **synapse-mlx-olap** — next session, after confirming candle-core dep budget
3. **synapse-graph** — after entity-graph schema design

## Conflict notes

- arrow v53 (tsdb) vs polars 0.46: polars uses `arrow2` crate tree, not `arrow` (apache arrow-rs). **No conflict.**
- cranelift 0.131 (market) vs synapse-jit 0.131: same version, but market already links directly — adding synapse-jit as dep would create a duplicate symbol path. **Skip.**
- io_uring: Linux-only, compile-only on macOS. **No runtime value.**
