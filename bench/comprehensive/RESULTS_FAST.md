# Synapse Comprehensive Benchmark Results
> Generated: 2026-04-25 14:13  |  Run type: **fast**

## Phase A — Bulk Insert

| Engine | ops/sec | Elapsed (s) | Disk MB | RSS MB | CPU% |
|--------|--------:|------------:|--------:|-------:|-----:|
| chromadb | 1,457 | 6.9 | 53.4 | 1,095 | 266 |
| duckdb | 111 | 90.0 | 9.4 | 2,024 | 241 |
| lancedb | 64,759 | 0.2 | 43.1 | 1,029 | 76 |
| qdrant | 6,964 | 1.4 | — | 1,074 | 80 |
| sqlite-vec | 35,638 | 0.3 | 31.4 | 917 | 59 |
| synapse | 47,050 | 0.2 | 31.4 | 948 | 50 |

## Phase B — Update (random 1k rows)

| Engine | ops/sec | Elapsed (s) | RSS MB | CPU% |
|--------|--------:|------------:|-------:|-----:|
| chromadb | 387 | 1.29 | 1,095 | 552 |
| duckdb | 293 | 1.71 | 2,016 | 143 |
| lancedb | — | — | — | — |
| qdrant | 6,499 | 0.08 | 1,077 | 30 |
| sqlite-vec | 10,626 | 0.05 | 921 | 19 |
| synapse | 9,358 | 0.05 | 952 | 22 |

## Phase C — Mixed 80/20 Read/Write (60s)

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| chromadb | 64.6 | 9.7 | 23.9 | 26.1 |
| duckdb | 158.6 | 6.2 | 10.5 | 14.6 |
| lancedb | — | — | — | — |
| qdrant | 44.0 | 23.7 | 47.0 | 52.5 |
| sqlite-vec | 278.4 | 4.4 | 5.4 | 8.3 |
| synapse | 283.6 | 4.3 | 4.9 | 6.8 |

## Phase D — 8-Thread Concurrent Select

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| chromadb | 224.8 | 33.9 | 58.6 | 86.7 |
| duckdb | 1655.2 | 3.6 | 11.2 | 19.8 |
| lancedb | — | — | — | — |
| qdrant | 33.6 | 235.7 | 397.5 | 489.8 |
| sqlite-vec | 55942.6 | 0.0 | 0.0 | 0.0 |
| synapse | 46980.8 | 0.0 | 0.0 | 0.0 |

## Overhead Analysis (CPU + RAM per 1k ops)

| Engine | RAM/1k-inserts (MB) | CPU%-insert | p99-concurrent (ms) |
|--------|--------------------:|------------:|--------------------:|
| chromadb | 109.54 | 266 | 86.7 |
| duckdb | 202.40 | 241 | 19.8 |
| lancedb | 102.95 | 76 | — |
| qdrant | 107.39 | 80 | 489.8 |
| sqlite-vec | 91.74 | 59 | 0.0 |
| synapse | 94.77 | 50 | 0.0 |

## Phase E — Recall@10 (vs brute-force cosine)

| Engine | Recall@10 | Queries |
|--------|----------:|--------:|
| chromadb | 0.920 | 50 |
| duckdb | 0.000 | 50 |
| lancedb | — | — |
| qdrant | 0.000 | 50 |
| sqlite-vec | 0.112 | 50 |
| synapse | 0.096 | 50 |

## Phase F — Concurrency Sweep (ops/sec vs threads)

| Engine | 1T ops/s | 4T ops/s | 8T ops/s | 16T ops/s | 16T p99ms |
|--------|----------:|---------:|---------:|----------:|----------:|
| chromadb | — | — | — | — | — |
| duckdb | — | — | — | — | — |
| lancedb | — | — | — | — | — |
| qdrant | — | — | — | — | — |
| sqlite-vec | — | — | — | — | — |
| synapse | — | — | — | — | — |

## Phase G — Batch Update Sweep (ops/sec vs batch size)

| Engine | batch=1 | batch=100 | batch=1000 |
|--------|--------:|----------:|-----------:|
| chromadb | — | — | — |
| duckdb | — | — | — |
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
