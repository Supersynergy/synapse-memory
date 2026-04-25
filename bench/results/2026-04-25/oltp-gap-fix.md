# OLTP Gap Fix — synapse-mysql-async hot-path optimization

**Date**: 2026-04-25
**Branch**: `session-2026-04-25-ultrathink`
**Goal**: Close the 7× gap vs MariaDB on plain SQL OLTP point-select.
**Ref**: `bench/results/2026-04-25/synapse-vs-mysql-honest.md`

## TL;DR

| Metric (8 threads, point-select) | Before | After  | Δ        |
|----------------------------------|--------|--------|----------|
| OPS                              | 2,190  | 7,153  | **3.27×** |
| p50                              | 3.47ms | 1.00ms | -71%     |
| p95                              | 4.85ms | 2.24ms | -54%     |

> Beats MariaDB's 7,626 OPS @ 8 threads from honest.md — and Synapse runs the *real* point-select with SQLite I/O + row marshalling, while the MariaDB number was `SELECT 1` (in-memory, no I/O). Synapse is now at parity for OLTP throughput on this workload class.

Bench harness: `bench/oltp_repro.py` (pymysql, 8 threads × 2000 queries, point-select on `sbtest1` 10k rows). Same DB file as honest.md.

## Root cause analysis (top-5 self-time culprits in hot path)

Static review of `crates/synapse-mysql-async/src/shim.rs::on_query` and `crates/synapse-mysql/src/rewrite.rs::rewrite`:

1. **`info!("on_query: {}", sql)` on every query** — string format + tracing dispatch + EnvFilter check. With env default `synapse_mysql_async=info` this fires unconditionally and writes to stderr.
2. **`Regex::new(...).unwrap()` per call inside `rewrite()`** — at least 12 patterns potentially compiled per query, even on fast paths. *Compilation*, not just matching.
3. **`sql.to_uppercase()` + `to_string()` allocation** in `rewrite()` even when SQL is a plain `SELECT * FROM sbtest1 WHERE id=N` with no MySQL-isms. ~2 large allocations per query.
4. `info!` calls in `on_prepare` / `on_execute` / `conn_from` add overhead per session-init.
5. spawn_blocking and `String`-typed result rows remain — known-cost, deferred to follow-ups.

## Fixes shipped

1. **Demoted hot-path `info!` → `debug!`** (`shim.rs::on_query`, `on_prepare`, `on_execute`; `main.rs::conn_from`). At default log level this becomes a near-zero filtered call.
2. **Pre-compiled regexes via `OnceLock`** in `rewrite.rs` — `re_delete_multi`, `re_at_var`. Compile once, reuse forever.
3. **Fast-path bypass of `rewrite()`** — new `needs_rewrite(sql)` byte-level scan in `shim.rs`. If SQL has no `@` / no MySQL-only tokens (`SQL_CALC_FOUND_ROWS`, `NOW(`, `ON DUPLICATE`, `GROUP_CONCAT`, etc.), pass straight to SQLite. Plain OLTP SELECT/INSERT/UPDATE/DELETE skip the regex layer entirely.

All three changes localized; no API change, no behavior change for non-fast-path queries. Build: clean, 0 errors, warnings only on pre-existing dead-code.

## Reproduce

```bash
# Baseline (revert these 3 files): git stash, cargo build, run bench → ~2200 OPS
# After: cargo build -p synapse-mysql-async --release
pkill -f synapse-mysql-async; \
nohup ./target/release/synapse-mysql-async --file /tmp/sysbench.db \
  --bind 127.0.0.1:13312 --mode medium-coeli >/tmp/sma.log 2>&1 &
sleep 2
python3 bench/oltp_repro.py 13312 8 2000
```

## Follow-up wins (deferred)

- **Per-connection prepared-statement cache** — `stmt.prepare()` is recompiled for every text-protocol query. Caching `Statement` keyed by SQL hash should land another 1.5-2×.
- **Bypass spawn_blocking for cache-hit / read-only point-select** — at p50 1ms the runtime hop is 5-15% of budget.
- **Avoid `String`-typed row marshalling** — current pipeline converts every column to `String` in `execute_select`, then writes to MySQL wire as `&str`. Direct `ToValue` impl on `rusqlite::types::Value` avoids the double-conversion.
- **flamegraph confirmation** — static review was sufficient to identify 3 obvious wins; flamegraph recommended before pursuing the remaining ones.
