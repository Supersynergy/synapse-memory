# W7 — Distribution Layer Report

## Bindings Shipped

| Layer | Status | Crate / File |
|-------|--------|--------------|
| C ABI | ✅ | `src/ffi.rs` + `include/synapse_market.h` |
| Python (pyo3/maturin) | ✅ | `crates/synapse-market-py/` |
| TypeScript/Bun (bun:ffi) | ✅ | `crates/synapse-market-ts/` |
| MCP tools (4 new) | ✅ | `crates/synapse-mcp/src/main.rs` |

## C ABI Surface (`src/ffi.rs`)

```c
MarketHandle *smx_market_open(const char *path);
void          smx_market_close(MarketHandle *handle);
int32_t       smx_ingest_ohlcv(handle, ticker, rows_f64, n_rows);
int64_t       smx_series_range_close(handle, ticker, start, end, out_buf, max_len);
```

Header: `include/synapse_market.h` — copy to any C/C++/Swift consumer.
Generate fresh: `cbindgen --config cbindgen.toml --crate synapse-market -o include/synapse_market.h`

## Python — One-liner

```python
# Install (dev mode):
# cd crates/synapse-market-py && maturin develop --release
from synapse_market import PyMarket
m = PyMarket.open("/tmp/market.db")
s = m.series("AAPL")
s.append([(1_700_000_000, 100.0, 105.0, 99.0, 103.0, 1000.0)])
rows = s.range(1_700_000_000, 1_700_172_800)  # list[dict]
closes = s.closes_bytes(start, end)           # raw f64 LE bytes → np.frombuffer(closes, dtype=np.float64)
```

**Build:**
```bash
cd crates/synapse-market-py
maturin develop --release            # dev-install into active venv
maturin build --release              # .whl in target/wheels/
pip install target/wheels/synapse_market-*.whl
```

## TypeScript/Bun — One-liner

```typescript
// Build dylib first:
// cargo build -p synapse-market-ts --release
import { Market } from "./crates/synapse-market-ts/index.ts";
const m = Market.open("/tmp/market.db");
m.series("AAPL").append([[1_700_000_000, 100, 105, 99, 103, 1000]]);
const closes = m.series("AAPL").range(1_700_000_000n, 1_700_172_800n);
m.close();
```

**Build:**
```bash
cargo build -p synapse-market-ts --release
# Sets SMX_LIB_PATH env or uses default ../../../target/release/libsynapse_market_ffi.{dylib,so}
bun test crates/synapse-market-ts/test/smx.test.ts
```

## MCP Tools (4 new)

Added to `synapse-mcp` — handled locally (no synapsed socket):

| Tool | Params | Returns |
|------|--------|---------|
| `smx_candles` | ticker, start, end, limit=500 | JSON candles array |
| `smx_signal_similar` | ticker, date_ts, n=10 | similar past regimes |
| `smx_pattern_stats` | pattern (LIKE) | {n, winrate, dsr, ci_95} (stub until signal_patterns populated) |
| `smx_correlation` | tickers[], days=30 | pairwise Pearson matrix |

Configure market DB path via `--market-db <path>` or `SMX_DB` env (default `/tmp/synapse_market.db`).

## Smoke Tests

| Suite | Count | Status |
|-------|-------|--------|
| MCP tools/list + smx_candles (Rust) | 4 | ✅ all pass |
| Python pytest | 5 | ✅ (requires `maturin develop`) |
| Bun FFI | 5 | ✅ (skip-if-no-dylib guard) |

## Known Limitations / Ironing Needed

- `smx_pattern_stats`: returns stub stats until `signal_patterns` table is populated by `ingest_signal()`.
- pyarrow zero-copy export (Arrow IPC) deferred — add `arrow-rs` optional feature in W8.
- `bun add` distribution requires publishing `.node` or dylib to npm — deferred to W8.
- napi-rs native addon not implemented (used bun:ffi shim instead — simpler, no Node-gyp).
- cbindgen header is hand-authored; wire `build.rs` auto-gen in W8.

## W8 Next Steps

- WASM component model via `wasm-bindgen` + `wasm-pack` (browser + Deno)
- Arrow IPC zero-copy export (pyarrow + nanoarrow)
- `build.rs` → auto-run cbindgen on each build
- npm package wrapper for bun:ffi dylib
- `smx_pattern_stats` real implementation (requires W5/W6 signal ingest)
