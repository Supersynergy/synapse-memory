# synapse-market Quickstart

Three languages, copy-paste runnable. Each block is self-contained.

---

## Rust

```toml
# Cargo.toml
[dependencies]
synapse-market = { path = "../../crates/synapse-market" }
anyhow = "1"
```

```rust
// src/main.rs
use synapse_market::{Market, Bar};

fn main() -> anyhow::Result<()> {
    // Open (creates if absent)
    let m = Market::open("/tmp/quickstart.db")?;

    // Ingest OHLCV bars
    let mut s = m.series("AAPL")?;
    s.append(&[
        Bar { ts: 1_700_000_000, o: 100.0, h: 105.0, l: 99.0, c: 103.0, v: 1_000.0 },
        Bar { ts: 1_700_000_900, o: 103.0, h: 107.0, l: 102.0, c: 106.0, v: 1_200.0 },
    ])?;

    // Range query
    let bars = s.range(1_700_000_000, 1_700_172_800)?;
    println!("fetched {} bars", bars.len());
    Ok(())
}
```

```bash
cargo build --release
cargo run --release
# or run the bundled demo:
cargo run --example backtest_demo -p synapse-market --release
```

---

## Python

Requires Python ≥3.10, maturin ≥1.4.

```bash
# One-time build (dev-install into active venv)
cd crates/synapse-market-py
maturin develop --release

# or build a distributable wheel:
maturin build --release
pip install target/wheels/synapse_market-*.whl
```

```python
from synapse_market import PyMarket
import numpy as np

# Open / create
m = PyMarket.open("/tmp/quickstart.db")
s = m.series("AAPL")

# Ingest: list of (ts, open, high, low, close, volume)
s.append([
    (1_700_000_000, 100.0, 105.0, 99.0, 103.0, 1_000.0),
    (1_700_000_900, 103.0, 107.0, 102.0, 106.0, 1_200.0),
])

# Structured query → list[dict]
bars = s.range(1_700_000_000, 1_700_172_800)
print(f"fetched {len(bars)} bars: {bars[0]}")

# Raw bytes → NumPy (zero-copy path)
raw = s.closes_bytes(1_700_000_000, 1_700_172_800)
closes = np.frombuffer(raw, dtype=np.float64)
print("closes:", closes)
```

---

## TypeScript / Bun

Requires Bun ≥1.1 and the compiled dylib.

```bash
# Build dylib first (once)
cargo build -p synapse-market-ts --release
# dylib lands at: target/release/libsynapse_market_ffi.dylib  (macOS)
#                 target/release/libsynapse_market_ffi.so     (Linux)

# Run tests
bun test crates/synapse-market-ts/test/smx.test.ts
```

```typescript
// quickstart.ts
import { Market } from "./crates/synapse-market-ts/index.ts";

const m = Market.open("/tmp/quickstart.db");
const s = m.series("AAPL");

// Ingest: [ts, open, high, low, close, volume]
s.append([
  [1_700_000_000, 100, 105, 99, 103, 1_000],
  [1_700_000_900, 103, 107, 102, 106, 1_200],
]);

// Range query → close prices
const closes = s.range(1_700_000_000n, 1_700_172_800n);
console.log("closes:", closes);

m.close();
```

```bash
bun run quickstart.ts
```

---

## MCP (Claude / Cursor)

Add to your MCP config, then use tool calls:

```json
{
  "mcpServers": {
    "synapse-market": {
      "command": "synapse-mcp",
      "args": ["--market-db", "/tmp/market.db"]
    }
  }
}
```

Available tools: `smx_candles`, `smx_signal_similar`, `smx_pattern_stats`, `smx_correlation`.
