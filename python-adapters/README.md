# Synapse-X Python Adapters

Three adapters that plug Synapse Market (`synapse-market-py`) into popular quant frameworks.

| Adapter | Framework | Status |
|---------|-----------|--------|
| `synapse_vectorbt` | vectorbt ≥0.27 | ✅ shipped |
| `synapse_qlib` | Qlib ≥0.9 | ✅ shipped (qlib optional) |
| `synapse_nautilus` | NautilusTrader ≥1.190 | ✅ shipped (nautilus optional) |

## Quick install

```bash
./install_all.sh
```

## Build synapse-market-py first

```bash
cd ../../crates/synapse-market-py
maturin develop --release
```

## Roadmap

- Live streaming via `SynapseDataClient.subscribe()`
- Qlib `Alpha158` feature set wired to Synapse derived-series
- vectorbt `from_synapse_multi()` for portfolio of tickers
