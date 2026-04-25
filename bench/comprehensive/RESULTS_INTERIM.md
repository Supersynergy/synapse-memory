# Synapse Comprehensive Benchmark Results
> Generated: 2026-04-25 13:43  |  Run type: **full**

## Phase A — Bulk Insert

| Engine | ops/sec | Elapsed (s) | Disk MB | RSS MB | CPU% |
|--------|--------:|------------:|--------:|-------:|-----:|
| sqlite-vec | 38,545 | 2.6 | 306.2 | 1,302 | 91 |

## Phase B — Update (random 1k rows)

| Engine | ops/sec | Elapsed (s) | RSS MB | CPU% |
|--------|--------:|------------:|-------:|-----:|
| sqlite-vec | 3,824 | 0.26 | 1,306 | 53 |

## Phase C — Mixed 80/20 Read/Write (60s)

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| sqlite-vec | 42.5 | 29.6 | 34.1 | 36.3 |

## Phase D — 8-Thread Concurrent Select

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| sqlite-vec | 55857.7 | 0.0 | 0.0 | 0.0 |

## Overhead Analysis (CPU + RAM per 1k ops)

| Engine | RAM/1k-inserts (MB) | CPU%-insert | p99-concurrent (ms) |
|--------|--------------------:|------------:|--------------------:|
| sqlite-vec | 13.02 | 91 | 0.0 |

## How to re-run

```bash
# Dry run (1k docs, ~2 min)
DRY_RUN=1 bash bench/comprehensive/run.sh

# Full run with all extended phases (100k docs)
PHASES=all setsid nohup bash bench/comprehensive/run.sh > bench/comprehensive/run.log 2>&1 &
tail -f bench/comprehensive/run.log
```
