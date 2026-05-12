# synapse-market

Embedded HFT/backtest engine on Synapse: OHLCV timeseries + regime-vec + news-FTS + graph-entity.
Columnar mmap pages + SIMD/AMX analytics + C/Python/TS bindings — no daemon required.

Beats SQLite ~4× on cold candle-range scans today (architecture proven; held-open path targets 10-15×).
AMX Pearson corr-matrix: **117× faster than naive, 26× faster than NEON** on M4 Max.
Plan-cache for adaptive operator selection: greenfield (DataFusion and DuckDB both lack it).

---

## Quickstart

### Rust

```rust
use synapse_market::{Market, Bar};

fn main() -> anyhow::Result<()> {
    let m = Market::open("/tmp/market.db")?;
    let mut s = m.series("AAPL")?;
    s.append(&[Bar { ts: 1_700_000_000, o: 100.0, h: 105.0, l: 99.0, c: 103.0, v: 1000.0 }])?;
    let bars = s.range(1_700_000_000, 1_700_172_800)?;
    println!("{} bars", bars.len());
    Ok(())
}
```

```bash
cargo build -p synapse-market --release
cargo run --example backtest_demo
```

### Python

```python
# Install: cd crates/synapse-market-py && maturin develop --release
from synapse_market import PyMarket
import numpy as np

m = PyMarket.open("/tmp/market.db")
s = m.series("AAPL")
s.append([(1_700_000_000, 100.0, 105.0, 99.0, 103.0, 1000.0)])
rows = s.range(1_700_000_000, 1_700_172_800)          # list[dict]
closes = s.closes_bytes(1_700_000_000, 1_700_172_800)  # raw f64 LE bytes
arr = np.frombuffer(closes, dtype=np.float64)
```

```bash
cd crates/synapse-market-py
maturin develop --release    # dev-install into active venv
# or: maturin build --release && pip install target/wheels/synapse_market-*.whl
```

### TypeScript (Bun)

```typescript
// Build dylib first: cargo build -p synapse-market-ts --release
import { Market } from "./crates/synapse-market-ts/index.ts";

const m = Market.open("/tmp/market.db");
m.series("AAPL").append([[1_700_000_000, 100, 105, 99, 103, 1000]]);
const closes = m.series("AAPL").range(1_700_000_000n, 1_700_172_800n);
m.close();
```

```bash
cargo build -p synapse-market-ts --release
bun test crates/synapse-market-ts/test/smx.test.ts
```

---

## Numbers

All measured on Apple M4 Max, macOS 15.5.

| Workload | synapse-market | Baseline | Speedup | Source |
|---|---:|---:|---:|---|
| Candle-range 2880 bars p50 (cold re-open) | 50µs | 193µs SQLite ROWID | **3.9×** | W1 bench |
| Candle-range expected (held-open handle) | ~5–15µs | 193µs SQLite | **~13–38×** (not benched yet) | W1 analysis |
| SIMD mean(close) A1 vs naive | — | — | gate ≥4× | W3 pass gate |
| Pearson corr 220×220 (AMX cblas_sgemm) | 25µs p50 | 3.5ms naive | **117×** | M4MAX-AMX-CORR-2026 |
| Pearson corr 220×220 (NEON f32×8) | 793µs | 3.5ms naive | **4.4×** | M4MAX-AMX-CORR-2026 |
| AMX vs NEON | 25µs | 793µs | **26×** | M4MAX-AMX-CORR-2026 |
| Plan-cache greenfield | n/a | n/a | no competitor | SYNAPSE-X-SOTA-2026 |

Numbers not yet benched: W2 held-open retune, W4 signal-similar, W5 window functions, W6 KG traversal, W7 mixed-workload router.

---

## Architecture

```
crates/synapse-market/
├── src/
│   ├── store/       # columnar mmap pages, delta-encoded ts (i32), f32 OHLCV, 64KB pages
│   ├── series/      # append-log + range scan, held-open Series handle
│   ├── analytics/   # SIMD (wide f32x8 / NEON) + AMX cblas_sgemm, window fns
│   ├── signal/      # regime-vec, similarity top-N via SimSIMD
│   ├── router/      # adaptive query-plan picker (plan-cache, bandit-routed)
│   ├── book/        # order-book replay
│   ├── ffi.rs       # C ABI: smx_market_open / smx_ingest_ohlcv / smx_series_range_close
│   ├── backtest.rs
│   ├── news.rs
│   ├── ohlcv.rs
│   └── regime.rs
├── include/
│   └── synapse_market.h   # C header (cbindgen)
├── benches/               # criterion benches: w1/w3/amx_minimal/router_mixed/book_replay
└── crates/
    ├── synapse-market-py/ # pyo3/maturin Python wheel
    └── synapse-market-ts/ # bun:ffi TypeScript bindings
```

Key design choices:
- **Zero-copy mmap** — sequential scan, no B-tree traversal, no kernel copy per row.
- **Delta-encoded timestamps** — i32 deltas halve timestamp storage vs i64.
- **f32 OHLCV** — 2× memory bandwidth vs f64, sufficient for quant workloads.
- **Inline AMX via Accelerate** — `cblas_sgemm` dispatches to AMX on M3+, zero extra deps.
- **No daemon required** — embedded single-binary; optional shared-state daemon for multi-process.

---

## Building

```bash
# Rust (lib + benches)
cargo build -p synapse-market --release
cargo nextest run -p synapse-market
cargo bench --bench amx_minimal -p synapse-market

# Python wheel
cd crates/synapse-market-py
maturin develop --release

# TypeScript / Bun
cargo build -p synapse-market-ts --release

# Regenerate C header
cbindgen --config cbindgen.toml --crate synapse-market -o include/synapse_market.h
```

---

## Milestone Status

| Sprint | Scope | Status |
|---|---|---|
| W1 | `store::page` + `store::column` + candle_range bench | ORANGE ⚠️ — 3.9× cold (re-open overhead), arch valid |
| W2 | `series::ingest` + held-open `series::range` + retune on real data | not benched yet |
| W3 | SIMD agg kernels (mean/VWAP/rolling/EWMA/corr) | gates pass (≥4× vs naive A1-A5) |
| W4 | `signal::similar` top-N via SimSIMD | not benched yet |
| W5 | `analytics::neon` + window functions | not benched yet |
| W6 | `kg` triples + BFS traversal | not benched yet |
| W7 | Distribution: C ABI + Python (pyo3) + TS (bun:ffi) + 4 MCP tools | GREEN ✅ |
| AMX | Pearson corr-matrix 220×220 cblas_sgemm | GREEN ✅ — 117× naive, 26× NEON |
| Plan-cache | Adaptive operator selection (bandit-routed) | Greenfield — no embedded competitor |

---

## Anti-Patterns

- **< 1k bars/ticker**: use SQLite. mmap page overhead isn't worth it below this threshold.
- **Shared multi-process OLTP**: use PostgreSQL. synapse-market is embedded-first.
- **Pure ad-hoc SQL**: use DuckDB. synapse-market wins on hot-path repeated queries, not exploration.
- **Non-Apple targets with AMX**: AMX path is Accelerate-only; NEON fallback auto-selects on Linux aarch64, scalar on x86.

---

## License

MIT — see workspace `Cargo.toml`.
