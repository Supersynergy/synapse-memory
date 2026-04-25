# Synapse Comprehensive Benchmark Results
> Generated: 2026-04-25 20:28  |  Run type: **fast**

## Phase A — Bulk Insert

| Engine | ops/sec | Elapsed (s) | Disk MB | RSS MB | CPU% |
|--------|--------:|------------:|--------:|-------:|-----:|
| milvus | 5,951 | 1.7 | — | 1,087 | 40 |
| pgvector | ERR | — | — | — | — |
| sqlite-vec | 14,779 | 0.7 | 31.4 | 917 | 26 |
| synapse | 15,330 | 0.7 | 31.4 | 969 | 27 |

## Phase B — Update (random 1k rows)

| Engine | ops/sec | Elapsed (s) | RSS MB | CPU% |
|--------|--------:|------------:|-------:|-----:|
| milvus | 4,229 | 0.12 | 1,089 | 22 |
| pgvector | — | — | — | — |
| sqlite-vec | — | — | — | — |
| synapse | — | — | — | — |

## Phase C — Mixed 80/20 Read/Write (60s)

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| milvus | 275.8 | 2.4 | 8.9 | 17.3 |
| pgvector | — | — | — | — |
| sqlite-vec | — | — | — | — |
| synapse | — | — | — | — |

## Phase D — 8-Thread Concurrent Select

| Engine | ops/sec | p50 ms | p95 ms | p99 ms |
|--------|--------:|-------:|-------:|-------:|
| milvus | 737.0 | 7.7 | 29.6 | 61.0 |
| pgvector | — | — | — | — |
| sqlite-vec | — | — | — | — |
| synapse | — | — | — | — |

## Overhead Analysis (CPU + RAM per 1k ops)

| Engine | RAM/1k-inserts (MB) | CPU%-insert | p99-concurrent (ms) |
|--------|--------------------:|------------:|--------------------:|
| milvus | 108.66 | 40 | 61.0 |
| pgvector | — | — | — |
| sqlite-vec | 91.74 | 26 | — |
| synapse | 96.92 | 27 | — |

## Errors

- **pgvector**: `pgvector SKIPPED: pg_isready not found — PostgreSQL not installed`

## How to re-run

```bash
# Dry run (1k docs, ~2 min)
DRY_RUN=1 bash bench/comprehensive/run.sh

# Full run with all extended phases (100k docs)
PHASES=all setsid nohup bash bench/comprehensive/run.sh > bench/comprehensive/run.log 2>&1 &
tail -f bench/comprehensive/run.log
```
