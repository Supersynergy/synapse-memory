# Synapse Comprehensive Benchmark Results
> Generated: 2026-04-25 12:01  |  Run type: **dry**

## Phase A — Bulk Insert

| Engine | ops/sec | Elapsed (s) | Disk MB | RSS MB | CPU% |
|--------|--------:|------------:|--------:|-------:|-----:|
| chromadb | 939 | 1.1 | 9.2 | 941 | 53 |
| lancedb | 79,552 | 0.0 | 4.3 | 905 | 6 |
| sqlite-vec | 67,158 | 0.0 | 0.0 | 849 | 7 |
| synapse | 67,590 | 0.0 | 0.0 | 895 | 7 |

## Phase B — Update (random 1k rows)

| Engine | ops/sec | Elapsed (s) | RSS MB | CPU% |
|--------|--------:|------------:|-------:|-----:|
| chromadb | 359 | 2.79 | 951 | 461 |
| lancedb | 23 | 43.85 | 1,387 | 419 |
| sqlite-vec | 22,023 | 0.05 | 853 | 22 |
| synapse | 13,813 | 0.07 | 900 | 23 |

## Phase C — Mixed 80/20 Read/Write (60s)

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| chromadb | 231.0 | 1.3 | 16.8 | 24.8 |
| lancedb | 39.4 | 0.4 | 192.0 | 466.1 |
| sqlite-vec | 5309.0 | 0.2 | 0.3 | 0.3 |
| synapse | 5403.4 | 0.2 | 0.3 | 0.3 |

## Phase D — 8-Thread Concurrent Select

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| chromadb | 2110.6 | 3.5 | 5.7 | 7.8 |
| lancedb | 2933.2 | 1.5 | 7.4 | 11.4 |
| sqlite-vec | 113386.4 | 0.0 | 0.0 | 0.0 |
| synapse | 120134.4 | 0.0 | 0.0 | 0.0 |

## Overhead Analysis (CPU + RAM per 1k ops)

| Engine | RAM/1k-inserts (MB) | CPU%-insert | p99-concurrent (ms) |
|--------|--------------------:|------------:|--------------------:|
| chromadb | 941.18 | 53 | 7.8 |
| lancedb | 904.71 | 6 | 11.4 |
| sqlite-vec | 848.86 | 7 | 0.0 |
| synapse | 895.45 | 7 | 0.0 |

## How to re-run

```bash
# Dry run (1k docs, ~2 min)
DRY_RUN=1 bash bench/comprehensive/run.sh

# Full run (100k docs, ~20-40 min)
bash bench/comprehensive/run.sh > bench/comprehensive/run.log 2>&1 &
tail -f bench/comprehensive/run.log
```
