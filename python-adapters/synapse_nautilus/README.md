# synapse-nautilus

NautilusTrader LiveDataClient backed by Synapse Market.

## Install

```bash
uv pip install -e .
uv pip install "nautilus-trader>=1.190"  # optional — Cython build ~5 min
```

## Demo

```python
from synapse_nautilus import SynapseDataClient

client = SynapseDataClient(
    market_path="/path/to/market.smx",
    instruments=["AAPL"],
    start_time=1_700_000_000,
    end_time=1_709_000_000,
)
bars = client.replay()
for bar in bars:
    print(bar.instrument_id, bar.ts_event, bar.close)
```

## Blocker

`nautilus-trader>=1.190` requires Rust toolchain + Cython. Install separately:
```bash
uv pip install nautilus-trader
```
The adapter works in stub mode without nautilus (emits `SynapseBar` dataclass events).
