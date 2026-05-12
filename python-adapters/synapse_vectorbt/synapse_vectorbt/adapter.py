"""Synapse-X → vectorbt adapter.

Loads OHLCV data from a Synapse Market file and returns a pandas DataFrame
with DatetimeIndex suitable for direct use with vectorbt.Portfolio.from_signals()
or vbt.Portfolio.from_order_func().
"""
from __future__ import annotations

import pandas as pd
import numpy as np


def _load_rows(market_path: str, ticker: str, start: int, end: int) -> list[dict]:
    """Load raw OHLCV dicts from Synapse Market. Lazy-import so stub tests work."""
    try:
        from synapse_market import PyMarket  # type: ignore[import]
        m = PyMarket.open(market_path)
        s = m.series(ticker)
        return s.range(start, end)
    except ModuleNotFoundError as exc:
        raise ImportError(
            "synapse-market-py not installed. Run: "
            "cd crates/synapse-market-py && maturin develop --release"
        ) from exc


def from_synapse(
    market_path: str,
    ticker: str,
    start: int,
    end: int,
) -> pd.DataFrame:
    """Return OHLCV DataFrame indexed by UTC DatetimeIndex.

    Parameters
    ----------
    market_path : str
        Path to .smx market file (or ":memory:").
    ticker : str
        Instrument symbol.
    start, end : int
        Unix timestamp range (seconds, inclusive).

    Returns
    -------
    pd.DataFrame
        Columns: open, high, low, close, volume. Index: DatetimeIndex (UTC).
    """
    rows = _load_rows(market_path, ticker, start, end)
    if not rows:
        return pd.DataFrame(columns=["open", "high", "low", "close", "volume"])

    ts = [r["ts"] for r in rows]
    data = {
        "open": np.array([r["open"] for r in rows], dtype=np.float64),
        "high": np.array([r["high"] for r in rows], dtype=np.float64),
        "low": np.array([r["low"] for r in rows], dtype=np.float64),
        "close": np.array([r["close"] for r in rows], dtype=np.float64),
        "volume": np.array([r["volume"] for r in rows], dtype=np.float64),
    }
    index = pd.to_datetime(ts, unit="s", utc=True)
    return pd.DataFrame(data, index=index)
