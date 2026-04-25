# Synapse Comprehensive Benchmark Results
> Generated: 2026-04-25 19:20  |  Run type: **fast**

## Phase A — Bulk Insert

| Engine | ops/sec | Elapsed (s) | Disk MB | RSS MB | CPU% |
|--------|--------:|------------:|--------:|-------:|-----:|
| chromadb | 1,457 | 6.9 | 53.4 | 1,095 | 266 |
| lancedb | 81,993 | 0.1 | 27.6 | 1,188 | 61 |
| qdrant | 1,720 | 5.8 | — | 950 | 25 |
| sqlite-vec | 35,638 | 0.3 | 31.4 | 917 | 59 |
| synapse | 47,050 | 0.2 | 31.4 | 948 | 50 |

## Phase B — Update (random 1k rows)

| Engine | ops/sec | Elapsed (s) | RSS MB | CPU% |
|--------|--------:|------------:|-------:|-----:|
| chromadb | 387 | 1.29 | 1,095 | 552 |
| lancedb | 7,530 | 0.07 | 1,209 | 46 |
| qdrant | 2,838 | 0.18 | 954 | 38 |
| sqlite-vec | 10,626 | 0.05 | 921 | 19 |
| synapse | 9,358 | 0.05 | 952 | 22 |

## Phase C — Mixed 80/20 Read/Write (60s)

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| chromadb | 64.6 | 9.7 | 23.9 | 26.1 |
| lancedb | 156.8 | 6.2 | 8.1 | 14.6 |
| qdrant | 33.0 | 31.1 | 67.2 | 102.0 |
| sqlite-vec | 278.4 | 4.4 | 5.4 | 8.3 |
| synapse | 283.6 | 4.3 | 4.9 | 6.8 |

## Phase D — 8-Thread Concurrent Select

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| chromadb | 224.8 | 33.9 | 58.6 | 86.7 |
| lancedb | 315.4 | 24.2 | 39.4 | 49.5 |
| qdrant | 50.4 | 153.6 | 295.9 | 341.1 |
| sqlite-vec | 55942.6 | 0.0 | 0.0 | 0.0 |
| synapse | 46980.8 | 0.0 | 0.0 | 0.0 |

## Overhead Analysis (CPU + RAM per 1k ops)

| Engine | RAM/1k-inserts (MB) | CPU%-insert | p99-concurrent (ms) |
|--------|--------------------:|------------:|--------------------:|
| chromadb | 109.54 | 266 | 86.7 |
| lancedb | 118.76 | 61 | 49.5 |
| qdrant | 94.98 | 25 | 341.1 |
| sqlite-vec | 91.74 | 59 | 0.0 |
| synapse | 94.77 | 50 | 0.0 |

## Phase E — Recall@10 (vs brute-force cosine)

| Engine | Recall@10 | Queries |
|--------|----------:|--------:|
| chromadb | 0.536 | 50 |
| lancedb | 0.992 | 50 |
| qdrant | 0.932 | 50 |
| sqlite-vec | 1.000 | 50 |
| synapse | 1.000 | 50 |

## Phase F — Concurrency Sweep (ops/sec vs threads)

| Engine | 1T ops/s | 4T ops/s | 8T ops/s | 16T ops/s | 16T p99ms |
|--------|----------:|---------:|---------:|----------:|----------:|
| chromadb | — | — | — | — | — |
| lancedb | — | — | — | — | — |
| qdrant | — | — | — | — | — |
| sqlite-vec | — | — | — | — | — |
| synapse | — | — | — | — | — |

## Phase G — Batch Update Sweep (ops/sec vs batch size)

| Engine | batch=1 | batch=100 | batch=1000 |
|--------|--------:|----------:|-----------:|
| chromadb | — | — | — |
| lancedb | — | — | — |
| qdrant | — | — | — |
| sqlite-vec | — | — | — |
| synapse | — | — | — |

## How to re-run

```bash
# Dry run (1k docs, ~2 min)
DRY_RUN=1 bash bench/comprehensive/run.sh

# Full run with all extended phases (100k docs)
PHASES=all setsid nohup bash bench/comprehensive/run.sh > bench/comprehensive/run.log 2>&1 &
tail -f bench/comprehensive/run.log
```
