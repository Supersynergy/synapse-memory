# synapse-vectorbt

Load Synapse Market data into vectorbt.

## Install

```bash
uv pip install -e .
uv pip install "vectorbt>=0.27"
```

## Demo

```python
from synapse_vectorbt import from_synapse

df = from_synapse("/path/to/market.smx", "AAPL", 1_700_000_000, 1_709_000_000)
# df: DataFrame[open,high,low,close,volume] with DatetimeIndex(UTC)

import vectorbt as vbt
pf = vbt.Portfolio.from_signals(
    df["close"],
    entries=df["close"] > df["close"].shift(1),
    exits=df["close"] < df["close"].shift(1),
)
print(pf.stats())
```
