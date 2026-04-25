# OLTP Prepared-Statement Cache — measured, mostly null result

**Date**: 2026-04-25
**Branch**: `session-2026-04-25-ultrathink`
**Hypothesis (from `oltp-gap-fix.md` follow-up #1)**: per-connection LRU of `rusqlite::CachedStatement` lifts OLTP point-select from 7153 → 12000+ OPS (1.7×+).
**Result**: **+2% only.** Hypothesis was wrong for this workload. Documented for the record.

## TL;DR

| Metric (8 threads × 2000, point-select on sbtest1) | Before (commit 36f422d) | After (prepare_cached) | Δ |
|----------------------------------------------------|-------------------------|------------------------|---|
| OPS                                                | 7,153                   | 7,250 (avg of 4 runs)  | **+1.4%** |
| p50                                                | 1.00 ms                 | 0.98 ms                | -2% |
| p95                                                | 2.24 ms                 | 2.16 ms                | -4% |

Run-by-run after change: 7116 / 7167 / 7392 / 7321 OPS.

## Why the cache doesn't fire

The bench harness (`bench/oltp_repro.py`) sends **text-protocol** queries with the literal id embedded:

```python
cur.execute(f"SELECT * FROM sbtest1 WHERE id={rid}")
```

So the SQL text varies on every query (`...id=1`, `...id=2`, ...). `Connection::prepare_cached(sql)` keys the LRU on the full SQL string — every call is a cache **miss** that allocates a new `CachedStatement`, prepares it, runs it, and inserts/evicts. Net cost ≈ same as `prepare()` plus extra HashMap work.

The 1-2% gain we *do* see comes from a small number of repeated session-init queries (`SELECT @@version`, `SELECT 1`, etc.) plus marginal allocator behavior.

## What would actually unlock this win

Two paths, both real engineering work, not 5-minute swaps:

1. **Literal-stripping cache key** — parse `WHERE id={N}` into `WHERE id=?` *before* the cache lookup, then bind `N` at execute time. Effectively client-side parameterization for text-protocol clients. Risk: false-collapse of semantically distinct queries; need a careful tokenizer.
2. **Use the actual MySQL prepared-statement protocol** (`COM_STMT_PREPARE` / `COM_STMT_EXECUTE`) — `on_prepare` / `on_execute` are already wired. A pymysql client with `cursor.execute(sql, (id,))` and `binary_protocol=True` would exercise that path and hit the cache by stmt_id. The current harness explicitly avoids this — it tests the text path on purpose.

Both are 1-2 day projects. Neither was in scope for the cache swap.

## Change shipped anyway

The `prepare()` → `prepare_cached()` swap is **kept** in `crates/synapse-mysql-async/src/shim.rs` (functions `execute_select` and `execute_select_with_params`):

- It is correct (rusqlite handles cache lifetime per-Connection).
- It is harmless on misses (~negligible HashMap overhead).
- It will pay off the moment either of the two unlock paths above lands, or whenever clients repeat identical SQL strings on the same connection.
- It is a one-liner that the next perf engineer would have written anyway.

## Reproduce

```bash
cargo build -p synapse-mysql-async --release
pkill -f synapse-mysql-async
nohup ./target/release/synapse-mysql-async --file /tmp/sysbench.db \
  --bind 127.0.0.1:13312 --mode medium-coeli >/tmp/sma.log 2>&1 &
sleep 2
for i in 1 2 3 4; do python3 bench/oltp_repro.py 13312 8 2000; done
```

## Next OLTP follow-up (revised priority)

Demoted #1 → option **(1) literal-stripping cache key** is now the highest-leverage 1.5-2× win.
The other two `oltp-gap-fix.md` follow-ups (skip spawn_blocking on cache-hit RO; avoid `String` row marshalling) remain valid and unaffected.
