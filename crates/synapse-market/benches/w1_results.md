# W1 Candle-Range Bench

## Results

| impl | p50 µs | p95 µs | mean µs |
|---|---:|---:|---:|
| SQLite WITHOUT ROWID | 190 | 209 | 191 |
| Synapse-X mmap-pages | 53 | 62 | 54 |

**Speedup p50: 3.6×  (mean: 3.5×)**

Gate: ORANGE ⚠️

## Setup
- 100 tickers × 2880 bars (60d 15m candles)
- 1000 iterations, warm cache (SQLite page-cache=64MB, mmap warm)
- SQLite WITHOUT ROWID table, indexed on (ticker, ts)

## Notes
- Synapse-X: columnar mmap pages, delta-encoded ts (i32), f32 OHLCV
- Page size: 64KB, ~2728 bars/page
- SQLite scans B-tree leaf pages; mmap scans sequential memory
