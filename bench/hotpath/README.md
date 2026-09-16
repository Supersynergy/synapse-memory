# hotpath bench

Reproducible hot-path timing for `synx` on a throwaway brain.

```bash
bench/hotpath/hotpath.sh [--docs N=5000] [--wal-mb M=64] [--reps R=3] [--out results.json]
```

Env overrides: `SYNX`, `SYNAPSED` for binary paths. If no `synapsed` binary
is found, the warm section is skipped and reported as `null`.

## What it measures (median of `--reps` runs, ms)

| key | path |
|---|---|
| `should_context` | predicate only — never opens the store |
| `context_cold_no_daemon` | `SYNAPSE_NO_DAEMON=1 context`: `Store::open` + recall |
| `prime_cold_no_daemon` | cold session prime |
| `context_warm_daemon` | same call over the synapsed socket |
| `prime_warm_daemon` | warm prime |
| `context_cold_fat_wal` | cold **recovery** open: pad WAL to `--wal-mb` under a held connection, then `kill -9` + drop `-shm` so the next open rebuilds the wal-index over the whole log |
| `context_cold_after_maintain` | cold open after `synx maintain` checkpointed+truncated the WAL |

The WAL section demonstrates the `wal_autocheckpoint=0` failure mode fixed in
`2035a96b`. Routine opens attach to the existing shared wal-index and stay
cheap even with a fat WAL — the cliff is **crash recovery**: with `-shm`
gone, the first open scans the entire log. The bench reproduces that exact
case (kill -9 + remove shm), then `maintain` drains+truncates and the lean
baseline is re-measured. `wal_bytes` before/after is reported.

Docs are ingested with `--no-embed`, so timings cover the IO/open/recall path,
not embedding throughput. Numbers are machine-relative — compare ratios, not
absolutes, across hosts.

Latest committed results: `RESULTS.md` (regenerate with `--out` + paste table).
