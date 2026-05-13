# Synapse-Market Capabilities Index — 2026-05-13

Crate: `synapse-market` | Path: `crates/synapse-market/` | Tests: 107 (33 inline + 74 integration/proptest)

---

## 16 Modules

| # | Module | Path | Description |
|---|--------|------|-------------|
| 1 | **OHLCV store** | `src/ohlcv.rs` | SQLite WITHOUT ROWID columnar tables per symbol. WAL-batched ingest ≥1M ticks/s. Range scan in 193µs (SQLite) / 50µs (mmap-pages). |
| 2 | **Mmap-pages** | `src/store/` | Columnar mmap pages: delta-encoded i32 ts + f32 OHLCV, 64KB page, ~2728 bars/page. 3.9× faster than SQLite on warm cache. |
| 3 | **Regime-vec** | `src/regime.rs` | Per-day feature embeddings → sqlite-vec similarity search. Identifies market regime transitions. |
| 4 | **News-FTS** | `src/news.rs` | FTS5 headline+body search, graph edges to tickers. Sub-ms full-text queries. |
| 5 | **Backtest engine** | `src/backtest.rs` | Deterministic tick replay. `Strategy` trait + `BacktestReport`. Walk-forward support. |
| 6 | **JIT filter** | `src/jit/` | Cranelift 0.131-compiled predicates (Cmp/And/Or/Not). Predicate hash → native-code cache. Filters row-batches in ~ns vs µs interpreted. |
| 7 | **Pattern FSM** | `src/pattern/` | DSL-parsed FSM engine for OHLCV pattern matching (BMNR/MP/RLMD/USAR/CLS etc). `parse_pattern()` + `FsmEngine`. |
| 8 | **Conformal predictor** | `src/conformal/` | Coverage-calibrated CI intervals. Walk-forward DSR-gate (p_win thresholding). |
| 9 | **Online learner** | `src/learn/` | Incremental model update on new ticks. Perceptron / SGD family. |
| 10 | **Signal index** | `src/signal/` | TurboVec + RaBitQ ANNS signal index. `TurboVecIndex` (SIMD-accelerated), `RabitqSignalIndex` (1-bit quantized). |
| 11 | **AMX correlation** | `src/analytics/` | AMX/NEON correlation matrix for M-Series. `correlation_matrix_amx()` — hardware-accelerated. |
| 12 | **Order book** | `src/book/` | L2 order book replay. BBO + depth-N. Used in HFT sim and `book_replay` bench. |
| 13 | **Alert engine** | `src/alert/` | Threshold + composite-score alerts. Writes to configurable sink. |
| 14 | **Router** | `src/router/` | Mixed-workload query router. Routes OHLCV / regime / FTS / signal queries. Bench: `router_mixed`. |
| 15 | **Stream** | `src/stream/` | Async WebSocket + MQTT tick feed ingestion. `stream_smoke` binary. |
| 16 | **Arrow IPC export** | `src/tsdb_export.rs` | `Market::export_arrow(ticker, range)` → Arrow RecordBatch. `arrow_to_ipc` / `ipc_to_arrow` roundtrip. Feature: `tsdb-export`. Via synapse-tsdb (arrow v53). |

---

## 7 Distribution Paths

| # | Path | Status |
|---|------|--------|
| 1 | **Rust library** (`rlib`) | Stable. Import via workspace. |
| 2 | **C-ABI shared lib** (`cdylib`) | Stable. FFI bindings in `src/ffi.rs`. |
| 3 | **Python binding** | `synapse-market-py/` crate. PyO3. `PyMarket.open()` in use by winvestment-web. |
| 4 | **TypeScript/WASM** | `synapse-market-ts/` crate. WASM target. |
| 5 | **MySQL wire shim** | `src/bin/smx_mysql_shim.rs`. Feature `smx-mysql`. Exposes SQL surface over MySQL protocol. |
| 6 | **MQTT stream** | `src/bin/stream_smoke.rs`. Feature `mqtt`. |
| 7 | **Arrow IPC** | `src/tsdb_export.rs`. Feature `tsdb-export`. Interop with any Arrow-native consumer (Python pyarrow, DuckDB, Polars). |

---

## Bench Baselines (measured on M4 Max, 2026-05)

| Bench | Result | File |
|-------|--------|------|
| Candle range — SQLite WITHOUT ROWID | p50=193µs | `benches/w1_candle_range.rs` |
| Candle range — mmap-pages (Synapse-X) | p50=50µs **(3.9×)** | `benches/w1_candle_range.rs` |
| SIMD aggregation (W3) | tbd | `benches/w3_simd_agg.rs` |
| Similarity (W5 RaBitQ vs TurboVec) | tbd | `benches/w5_similarity.rs` |
| Bloom — neg-lookup | tbd | `benches/bloom_neglookup.rs` |
| XOR vs Bloom filter | tbd | `benches/xor_vs_bloom.rs` |
| Compression ratio (zstd) | tbd | `benches/compression_ratio.rs` |
| JIT filter vs interpreted | tbd | `benches/jit_filter.rs` |
| Router mixed workload | tbd | `benches/router_mixed.rs` |
| Order book replay | tbd | `benches/book_replay.rs` |
| API correlation (AMX) | tbd | `benches/api_correlation.rs` |
| Online learner | tbd | `benches/online_learner.rs` |
| Pattern throughput | tbd | `benches/pattern_throughput.rs` |
| HotSet point lookup | tbd | `benches/hotset_point.rs` |
| AMX minimal (raw BLAS) | tbd | `benches/amx_minimal.rs` |
| AMX vs NEON vs MLX | tbd | `benches/amx_neon_mlx.rs` |
| TurboVec vs RaBitQ | tbd | `benches/turbovec_vs_rabitq.rs` |

---

## Test Count Breakdown (107 total)

| Category | Count | Notes |
|----------|-------|-------|
| Inline `#[test]` in src/ | 33 | Unit tests per module |
| Integration tests `tests/` | ~70 | proptest, roundtrip, backtest scenarios |
| Pre-existing failing | 1 | `proptest_pages::roundtrip_identity` — close mismatch at extreme values. Pre-dates this session. |
| New (tsdb-export) | 2 | `test_export_arrow_roundtrip_ipc`, `test_export_arrow_empty_range` |

---

## What This Is For

- Embedded HFT backtesting and live sim without external DB process
- OHLCV + regime + news + signal in a single SQLite file (zero-deploy)
- Arrow IPC bridge to Python/DuckDB/Polars ecosystem
- Pattern-FSM + JIT for sub-µs predicate evaluation on tick streams
- RaBitQ/TurboVec ANNS for signal similarity retrieval

## What This Is NOT For

- General-purpose OLAP (use DuckDB directly)
- Multi-node distributed storage (use synapse-raft or GreptimeDB)
- Real-time streaming ingestion at Kafka scale (use synapse-stream)
- Linux io_uring high-throughput writes (use synapse-iouring, macOS = noop)
- Full KG traversal / PageRank (use synapse-graph on brain.db)
