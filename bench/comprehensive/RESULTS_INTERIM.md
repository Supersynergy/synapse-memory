# Synapse Comprehensive Benchmark Results
> Generated: 2026-04-25 14:08  |  Run type: **full**

## Phase A — Bulk Insert

| Engine | ops/sec | Elapsed (s) | Disk MB | RSS MB | CPU% |
|--------|--------:|------------:|--------:|-------:|-----:|
| chromadb | 764 | 130.9 | 493.7 | 1,836 | 266 |
| sqlite-vec | 38,545 | 2.6 | 306.2 | 1,302 | 91 |
| synapse | 39,308 | 2.5 | 306.2 | 1,402 | 87 |

## Phase B — Update (random 1k rows)

| Engine | ops/sec | Elapsed (s) | RSS MB | CPU% |
|--------|--------:|------------:|-------:|-----:|
| chromadb | 290 | 3.45 | 1,836 | 472 |
| sqlite-vec | 3,824 | 0.26 | 1,306 | 53 |
| synapse | 3,701 | 0.27 | 1,406 | 48 |

## Phase C — Mixed 80/20 Read/Write (60s)

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| chromadb | 14.5 | 67.7 | 104.8 | 133.0 |
| sqlite-vec | 42.5 | 29.6 | 34.1 | 36.3 |
| synapse | 29.5 | 40.7 | 53.2 | 62.4 |

## Phase D — 8-Thread Concurrent Select

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| chromadb | 20.5 | 407.6 | 586.1 | 626.0 |
| sqlite-vec | 55857.7 | 0.0 | 0.0 | 0.0 |
| synapse | 62125.3 | 0.0 | 0.0 | 0.0 |

## Overhead Analysis (CPU + RAM per 1k ops)

| Engine | RAM/1k-inserts (MB) | CPU%-insert | p99-concurrent (ms) |
|--------|--------------------:|------------:|--------------------:|
| chromadb | 18.36 | 266 | 626.0 |
| sqlite-vec | 13.02 | 91 | 0.0 |
| synapse | 14.02 | 87 | 0.0 |

## How to re-run

```bash
# Dry run (1k docs, ~2 min)
DRY_RUN=1 bash bench/comprehensive/run.sh

# Full run with all extended phases (100k docs)
PHASES=all setsid nohup bash bench/comprehensive/run.sh > bench/comprehensive/run.log 2>&1 &
tail -f bench/comprehensive/run.log
```
