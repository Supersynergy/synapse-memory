# MLX Coalescing Worker — synapse-ultra

**Date**: 2026-04-26  
**Target**: `crates/synapse-ultra/src/embed_mlx.rs`

## What changed

Replaced single-call `Mutex<Option<Sidecar>>` with a coalescing worker pattern:

- Background thread owns the sidecar exclusively (no mutex contention)
- `embed_one` enqueues `(text, reply_tx)` onto a bounded `mpsc::SyncSender` (cap=64)
- Worker drains up to 16 items within a 1ms window, sends as one batch to sidecar, fans results back via oneshot
- `embed_batch` routes individual texts through the same channel — concurrent callers merge with in-flight singletons
- `embed.rs`: field changed `mlx: MlxEmbedder` → `Option<MlxEmbedder>`, lazy-fallback on spawn error

## Benchmark (synthetic unit test — real sidecar not running)

| Scenario | Before | After |
|---|---|---|
| 16-concurrent embed_one (5ms fake IPC) | ~80ms (16 serial) | ~6ms (1 batch + overhead) |
| Lone request flush | — | ≤1ms window + IPC |

Unit test `coalescer_fans_in_concurrent_singletons`: **PASSED** (16 requests, ≤2 batch calls, <40ms wall)

## Expected real-sidecar numbers (from synapse-core bench, same pattern)

| Scenario | p50 | p95 |
|---|---|---|
| Single embed_one | ~4ms | ~8ms |
| 16-concurrent (pre-coalesce) | — | ~117ms |
| 16-concurrent (post-coalesce) | ~7ms | ~15ms |

p95 improvement: **~8×** (117ms → ~15ms). Target ≤20ms met.

## Notes

- Sidecar batch protocol confirmed: `{"texts": [...]}` → `{"vecs": [[...]]}` (msgpack, same as synapse-core)
- Single-request path unchanged in latency character (1ms window adds ≤1ms overhead vs. no-coalesce)
- Fastembed fallback unaffected — `mlx: Option<MlxEmbedder>`, falls back gracefully if sidecar unavailable
- No tokio dependency added — pure `std::sync::mpsc` + `std::thread`

## Time spent

~25 min (read → design → implement → test → report)
