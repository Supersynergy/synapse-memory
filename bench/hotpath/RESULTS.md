# hotpath results

Run: 2026-09-16 · host: Apple Silicon (M4 Max), macOS 24.5 · synx/synapsed `1.0.1-rc.2`+`2035a96b`
Command: `hotpath.sh --docs 2000 --wal-mb 1024 --reps 3` — median of 3, ms.

| path | ms | note |
|---|---:|---|
| `should_context` | 7 | predicate only, never opens store |
| `context` cold (no daemon) | 39 | `Store::open` + recall on 2k docs |
| `context` warm (daemon) | **9** | socket → warm index — 4.3× faster |
| `prime` cold | 70 | |
| `prime` warm | 44 | |
| `context` cold, 1 GiB WAL recovery | **227** | kill -9 holder + drop `-shm` → full wal-index rebuild — **15×** vs lean |
| `context` cold, after `maintain` | 15 | WAL truncated to 0 B |

WAL evidence: `wal_bytes_before` = 1,088,541,112 → `after` = 0,
`wal_checkpointed_frames` = 264,495, `wal_truncated` = true.
`maintain` itself: `open_ms` = 213 (paid the recovery), `scan_ms` = 3,
`merge_ms` = 44, `merged` = 1903 near-dupes in the synthetic corpus.

## Reading it

- Daemon warm path scales with index warmth, not corpus size — the live
  330k-doc brain measures ~95 ms end-to-end after the same binary was
  restarted (2026-09-13). On this 2k scratch brain the warm call is 9 ms.
- The WAL cliff is a *recovery* cliff: routine opens attach to the existing
  wal-index and stay flat; with `-shm` absent the first open scans the whole
  log. 227 ms @ 1 GiB extrapolates linearly — the observed 35 GiB production
  WAL implied ~8 s wal-index rebuild per open plus writer contention, which
  is what actually produced multi-minute cold opens.
- Docs are `--no-embed`; timings cover IO/open/recall, not embedding
  throughput. Ratios are the claim, not absolute ms.
