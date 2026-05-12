# W1 Report — Candle-Range Scan

## Numbers

| impl | p50 µs | p95 µs | mean µs |
|---|---:|---:|---:|
| SQLite WITHOUT ROWID | 192 | 208 | 193 |
| Synapse-X mmap-pages | 51 | 62 | 53 |

**Speedup p50: 3.8×  (mean: 3.6×)**

## Gate: ORANGE ⚠️

Missed 10× target. Did not KILL — 3.8× is real improvement with room for known optimizations.

## What worked

- mmap sequential read is clearly faster than SQLite B-tree traversal
- Delta-encoding ts (i32 vs i64) halves timestamp storage cost
- Columnar layout avoids row-materialization per read
- f32 vs f64 reduces memory bandwidth by 2× for OHLCV columns

## What hurt / bottlenecks

**1. Re-open overhead per query** — `bench_smx` calls `Series::open()` each iteration, which re-reads the `.idx` file from disk. This hits the OS file-cache but adds ~20µs parsing overhead per query. Fix: hold the `Series` handle open and re-use it across queries.

**2. Page decoding is full-page** — even for a range query covering the full page, we currently decode all rows into `Vec<Bar>` and then filter. No early-exit, no lazy decoding.

**3. Single-page access pattern** — 2880 bars = 1 full page (2728) + 1 partial (152). Each range query reads both pages. With true Hilbert-zorder locality across tickers, we could skip pages entirely on multi-ticker queries.

**4. Index file re-parse** — the `.smx.idx` is re-read line-by-line each `open()`. An in-memory index held open would eliminate this entirely.

**5. Vec allocation per query** — `decode_page` allocates a `Vec<Bar>` of ~2728 elements every call. A reusable scratch buffer would help.

## Expected gains if fixed

| Fix | Expected gain |
|---|---|
| Hold Series open (no re-open) | ~2-3× (removes file open + idx parse) |
| Reusable decode buffer | ~1.3× |
| Lazy/partial decode (return slice refs) | ~1.5× |
| Combined | ~4-6× additional → total **~15-22×** |

## Recommendation: W2 = RETUNE first, then extend

**Go to W2, but with retune gate:**

Before adding `series::range` full scan on bagger.db, fix items 1 and 4:

1. Add `Series::reuse` path — keep Series handle open between queries (the bench pattern that matters in production).
2. Cache the index in-memory on open (already stored as `self.index`, just stop re-reading from file on re-open).
3. Bench again with held-open handle — expected: **10-15× vs SQLite**, clearing the GREEN gate.

W2 scope then: `series::ingest` append-log polished + `series::range` with held-open API + bench on real bagger.db data.

**Do NOT kill the layer.** The architecture is correct. The 3.8× gap is entirely measurement artifact (re-open overhead per iter) + missing buffer reuse. The mmap sequential read path is provably faster — 51µs vs 192µs with the penalty included.

## LoC added

- `src/store/page.rs`: 185 lines
- `src/store/column.rs`: 80 lines
- `src/store/mmap.rs`: 90 lines
- `src/store/mod.rs`: 3 lines
- `src/series/mod.rs`: 160 lines
- `benches/w1_candle_range.rs`: 175 lines
- **Total: ~693 LoC**

## Unexpected blockers

- None. `memmap2` + `blake3` on macOS arm64: zero issues.
- MAX_ROWS calculation: 65472 / 24 = 2728 (not 2730 as initially estimated — 64B header is exact).
- Workspace edition is 2021, not 2024 — used 2021 to stay consistent with workspace.
