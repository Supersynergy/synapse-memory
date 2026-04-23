# Scale Ladder — 2026-04-23

**Host**: M4 Max 128GB · **dim**=384 · **Q**=100/cell · median-of-single ingest, percentiles over Q queries · 30s warmup
**Raw**: `scale_ladder.csv` + `scale_ladder.json` (same dir).

## p95 query latency (ms) by scale

| Engine | N=1,000 | N=10,000 | N=100,000 | N=1,000,000 |
|---|---|---|---|---|
| synapse | 0.19 | 0.27 | 0.26 | 0.28 |
| sqlite_vec | 0.31 | 2.32 | 24.44 | 271.84 |
| chroma | 0.32 | 0.50 | 0.75 | 0.63 |

## Ingest wall-clock (seconds) by scale

| Engine | N=1,000 | N=10,000 | N=100,000 | N=1,000,000 |
|---|---|---|---|---|
| synapse | 0.2 | 0.9 | 3.6 | 41.6 |
| sqlite_vec | 0.1 | 0.7 | 7.7 | 81.9 |
| chroma | 0.1 | 0.5 | 10.1 | 164.1 |

## Disk footprint (MB) by scale

| Engine | N=1,000 | N=10,000 | N=100,000 | N=1,000,000 |
|---|---|---|---|---|
| synapse | 1.9 | 17.8 | 174.3 | 1740.6 |
| sqlite_vec | 1.6 | 16.0 | 156.6 | 1562.1 |
| chroma | 4.1 | 28.9 | 195.4 | 1865.2 |