# synapse-qlib

Qlib DataHandler backed by Synapse Market.

## Install

```bash
uv pip install -e .
uv pip install "pyqlib>=0.9"   # optional — adapter works as plain fetcher without it
```

## Demo

```python
from synapse_qlib import QlibSynapseHandler

h = QlibSynapseHandler(
    market_path="/path/to/market.smx",
    instruments=["AAPL", "MSFT"],
    start_time=1_700_000_000,
    end_time=1_709_000_000,
)
df = h.fetch()
# df: MultiIndex(instrument, datetime) × [open,high,low,close,volume]
print(df.head())
```

## Blocker

`pyqlib>=0.9` requires `torch` + `lightgbm` + `mlflow`. Install separately:
```bash
uv pip install torch lightgbm mlflow pyqlib
```
The adapter itself imports fine without qlib (stub mode).
