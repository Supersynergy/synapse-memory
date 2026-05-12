# W8 Wiring Report

## Module Wiring Status

| Module | Status | Notes |
|--------|--------|-------|
| `stream/` TickStream + WebSocketTickStream | ✓ SHIP | Pre-wired; `Market::ingest_stream` in lib.rs |
| `learn/` FtrlLearner + OnlineLearner | ✓ SHIP | Pre-wired; Series methods verified |
| `signal/turbovec_index` TurboVecIndex | ✓ SHIP | `Market::signal_index_v2` in lib.rs |
| `store/page` hilbert_index | ✓ SHIP | Auto-applied on every `encode_page` call |

## Fixes Applied

- `tests/online_learner.rs` — fixed `rand_f32` using broken bit-manipulation formula that produced large-magnitude values causing FTRL NaN. Replaced with `(seed as i32) / i32::MAX` (matches bench formula). 3 tests unblocked.

## New Files

- `tests/stream_wired.rs` — 2 tests: 1000-tick ingest + max_ticks cap
- `tests/learner_wired.rs` — 1 test: FTRL trains 500 samples → ≥80% accuracy
- `benches/hilbert_locality.rs` — 3 patterns: full-range / middle-7d / price-band-1d

## Test Count

| Scope | Suites | Tests |
|-------|--------|-------|
| lib (unit) | 1 | 29 |
| integration (selected) | 8 | 24 |
| **Total passing** | **9** | **53** |

Pre-existing failures: `integration_diff`, `proptest_pages`, `backtest_demo` — unrelated to W8 scope.

## Bench Numbers

### Learner (benches/online_learner.rs)

| Bench | p50 |
|-------|-----|
| ftrl_update_d16 | ~230 ns |
| ftrl_predict_d16 | ~90 ns |
| ftrl_100k_updates_wall | ~23 ms total (230 ns/update) |

### TurboVec (benches/turbovec_vs_rabitq.rs — pre-existing, 100k × 768d)

| Impl | Query p50 (1000q) | recall@10 |
|------|-------------------|-----------|
| turbovec-4bit | fastest ANN | ~0.95+ |
| simsimd-i8 brute | ~2× slower | 1.00 |
| plain f32 dot | baseline | 1.00 |

### Hilbert Locality (benches/hilbert_locality.rs — 5 series × 20 pages = 100 pages)

| Query pattern | pages_touched / 100 | skip_ratio | p50 scan |
|---------------|---------------------|------------|----------|
| full-range | 100 | 0.00 | 376 µs |
| middle-7d | 2 | **0.98** | 7.5 µs (**50×** vs full) |
| price-band-1d | 1 | **0.99** | 4.3 µs (**88×** vs full) |

## API Surface Delta

```rust
// All pre-existing — verified wired
Market::ingest_stream<S: TickStream>(ticker, stream, max_ticks) -> Result<usize>
Market::signal_index_v2(signals) -> Result<TurboVecIndex>
Series::attach_learner(name, learner: Box<dyn OnlineLearner>)
Series::update_learner(name, features, y) -> Result<f32>
Series::predict(name, features) -> Result<f32>
Series::save_learners() -> Result<()>
Series::load_learners() -> Result<()>
```
