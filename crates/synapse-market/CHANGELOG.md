# Changelog

All notable changes to `synapse-market`.

## [Unreleased]

### Changed
- `src/stream/parser.rs` — live tick parser switched `serde_json` → `sonic-rs` (Value-based, behavior-preserving). All 4 wire formats (Generic/PolygonV3/TradierV1/KrakenV2).
- `benches/parser_sonic.rs` — criterion before/after (serde vs sonic), M4 Max median: generic 673→248ns (2.7×), tradier 505→279ns (1.8×), kraken 890→626ns (1.4×), polygon 457→348ns (1.3×). Flat objects gain most; array/nested less. Hot path = `stream/ws.rs:41` per ws-message.
- Oracle: existing 7 parser tests green after swap.

## [0.2.0] — 2026-05-12

### Added

**W1 — Store + Candle-Range Bench**
- `src/store/page.rs` — 64KB columnar mmap pages, 2728 bars/page (185 LoC)
- `src/store/column.rs` — f32 OHLCV, delta-encoded i32 timestamps (80 LoC)
- `src/store/mmap.rs` — memmap2 sequential reader, no kernel copy (90 LoC)
- `benches/w1_candle_range.rs` — criterion bench vs SQLite WITHOUT ROWID
- Result: 3.9× p50 cold (50µs vs 193µs); ORANGE gate — re-open overhead identified, architecture valid

**W3 — SIMD Aggregation Kernels**
- `src/analytics/` — mean, VWAP, rolling_mean_20, EWMA, Pearson corr (wide f32x8 / NEON)
- `benches/w3_simd_agg.rs` — A1–A5 pass gates (≥4× vs naive for A1/A2, ≥3× for A3/A5, ≥10× for A4)
- Stack estimate: 7.2× pipeline multiplier on A1 baseline

**W4 — AMX Pearson Corr-Matrix (M4 Max)**
- `benches/amx_minimal.rs` — cblas_sgemm via Accelerate dispatch to AMX coprocessor
- 220 tickers × 252 daily returns (f32): **117× vs naive (3.5ms→25µs p50), 26× vs NEON f32x8**
- 808.8 GFLOPS sustained; NEON baseline 30.7 GFLOPS; naive 6.9 GFLOPS
- `benches/amx_neon_mlx.rs` — extended W-A/B/C/D workload comparison
- Source: `docs/synapse-x-design/M4MAX-AMX-CORR-2026.md`

**Plan-Cache (Greenfield)**
- Verified: DataFusion has no plan-cache (ghgrep 0 hits); DuckDB has prepared-statement cache only
- Adaptive operator selection via bandit-routed plan-picker in `src/router/` — no embedded competitor
- Source: `docs/synapse-x-design/SYNAPSE-X-SOTA-2026.md`

**W7 — Distribution Layer**
- `src/ffi.rs` + `include/synapse_market.h` — C ABI (smx_market_open/close/ingest_ohlcv/series_range_close)
- `crates/synapse-market-py/` — pyo3/maturin Python wheel; PyMarket.open / s.range / s.closes_bytes
- `crates/synapse-market-ts/` — bun:ffi TypeScript bindings; Market.open / series / range
- 4 new MCP tools in `synapse-mcp`: smx_candles, smx_signal_similar, smx_pattern_stats, smx_correlation
- Smoke tests: 4 Rust MCP + 5 Python pytest + 5 Bun FFI — all green

### Changed
- Workspace edition kept at 2021 (consistent with parent workspace)
- `crate-type = ["cdylib", "rlib"]` — enables both C FFI and Rust library consumers

### Known Limitations
- `smx_pattern_stats` returns stub stats until `signal_patterns` table populated by `ingest_signal()`
- pyarrow zero-copy Arrow IPC export deferred (W8)
- `build.rs` cbindgen auto-gen deferred (W8)
- bun:ffi shim used instead of napi-rs (no Node-gyp)

---

## [0.1.0] — prior

### Added
- `src/ohlcv.rs` — SQLite-backed OHLCV ingestion and query
- `src/regime.rs` — 5-dim regime vector storage
- `src/news.rs` — FTS5-backed news ingest
- `src/backtest.rs` — backtest engine scaffold
- `src/book/` — order-book replay
- `examples/backtest_demo.rs` — end-to-end demo
