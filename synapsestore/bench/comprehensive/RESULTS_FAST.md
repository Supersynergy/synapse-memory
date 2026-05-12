# Comprehensive Bench Results -- Profile: fast

## Phase A -- Bulk Insert

| Engine | ops/sec | RSS MB | Disk MB | CPU% |
|--------|---------|--------|---------|------|
| chromadb | 2740 | 1120 | 53.4 | 400 |
| lancedb | 77305 | 1125 | 27.6 | 64 |
| qdrant | 7411 | 1020 | 0.0 | 71 |
| sqlite-vec | 52326 | 964 | 31.4 | 82 |
| synapse | 51517 | 927 | 31.4 | 85 |

## Phase F -- Concurrency Saturation Sweep

| Engine | 1T ops/s | 4T ops/s | 8T ops/s | 16T ops/s | 32T ops/s | 64T ops/s | Sat.Threads |
|--------|----------|----------|----------|-----------|-----------|-----------|-------------|
| chromadb | 797 | 2892 | 3888 | 3642 | 3600 | 3700 | 16 |
| lancedb | 333 | 629 | 637 | 615 | 563 | 536 | 8 |
| qdrant | 537 | 1458 | 1522 | 1558 | 2031 | 2068 | 8 |
| sqlite-vec | 288380 | 106217 | 104593 | 104627 | 104262 | 105350 | 4 |
| synapse | 289895 | 104065 | 104794 | 100809 | 106927 | 104188 | 4 |

## Phase G -- Parquet Pre-Embed Bulk Load (10k docs, no embedding compute)

| Engine | docs/sec | rss_delta MB | disk MB | wall_s | Notes |
|--------|----------|--------------|---------|--------|-------|
| sqlite-vec | 27,473 | 174 | 31.4 | 0.4 | batched executemany 10k rows |
| synapse | 27,852 | 136 | 31.4 | 0.4 | same as sqlite-vec (fallback) |
| duckdb | SKIP | — | — | — | BLOB→FLOAT[384] cast unimplemented |
| lancedb | TIMEOUT | — | — | — | Arrow schema mismatch on add() |
| qdrant | 7,374 | 234 | 0.0 | 1.4 | batch upsert 1000/chunk |
| chromadb | 395 | 246 | 61.9 | 25.3 | slowest; HNSW build during load |

**100k/sec threshold**: sqlite-vec (27k) and synapse (27k) are in the ballpark for small batches. None break 100k/sec at 10k doc scale (embedding-skip path). 1M/sec claim requires native binary/mmap path not exposed via Python.

## Phase H -- Group-Commit Batch Sweep (5000 inserts, batch sizes 1→10000)

| Engine | batch=1 | batch=10 | batch=100 | batch=1000 | batch=10000 | best_batch | gain (1→best) |
|--------|---------|----------|-----------|------------|-------------|-----------|---------------|
| sqlite-vec | 14,855 | 34,682 | 42,530 | 47,322 | **51,198** | 10000 | 3.4× |
| synapse | 15,046 | 34,358 | 42,717 | 41,328 | **49,420** | 10000 | 3.3× |
| duckdb | 193 | 177 | 199 | 232 | **264** | 10000 | 1.4× |
| lancedb | TIMEOUT | — | — | — | — | — | — |
| qdrant | 318 | 1,704 | 3,163 | **4,791** | 3,508 | 1000 | 15.1× |
| chromadb | 75 | 424 | 1,528 | **1,909** | 2,023 | 10000 | 26.9× |

**Group-commit validates**: Every engine except DuckDB shows significant gains from batch coalescing. Qdrant peaks at batch=1000 (network round-trips dominate at 10000). ChromaDB shows the largest relative gain (26.9×) — per-call overhead is massive.

## Phase I -- Cache Hitrate (1001 identical queries)

| Engine | first_ms | mean_repeat_ms | p50_ms | p99_ms | speedup |
|--------|----------|----------------|--------|--------|---------|
| chromadb | 3.491 | 1.0836 | 1.0718 | 1.3270 | 3.22x |
| lancedb | 33.158 | 3.1263 | 3.0261 | 4.6095 | 10.61x |
| qdrant | 2.622 | 1.7800 | 1.7313 | 3.0320 | 1.47x |
| sqlite-vec | 3.395 | 2.0356 | 1.9749 | 3.0205 | 1.67x |
| synapse | 2.491 | 2.0413 | 2.0033 | 2.5904 | 1.22x |
