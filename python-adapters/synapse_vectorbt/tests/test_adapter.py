"""Tests for synapse_vectorbt adapter — no synapse-market-py required."""
from __future__ import annotations

from unittest.mock import patch, MagicMock

import pandas as pd
import numpy as np
import pytest

from synapse_vectorbt.adapter import from_synapse

SYNTHETIC_ROWS = [
    {"ts": 1_700_000_000 + i * 60, "open": 100.0 + i, "high": 101.0 + i,
     "low": 99.0 + i, "close": 100.5 + i, "volume": 1000.0 + i}
    for i in range(5)
]


def _mock_market(rows):
    series = MagicMock()
    series.range.return_value = rows
    market = MagicMock()
    market.series.return_value = series
    PyMarket = MagicMock()
    PyMarket.open.return_value = market
    return PyMarket


def test_from_synapse_shape():
    PyMarket = _mock_market(SYNTHETIC_ROWS)
    with patch.dict("sys.modules", {"synapse_market": MagicMock(PyMarket=PyMarket)}):
        import importlib, synapse_vectorbt.adapter as mod
        with patch.object(mod, "_load_rows", return_value=SYNTHETIC_ROWS):
            df = from_synapse(":memory:", "AAPL", 0, 9_999_999_999)

    assert isinstance(df, pd.DataFrame)
    assert df.shape == (5, 5)
    assert list(df.columns) == ["open", "high", "low", "close", "volume"]


def test_from_synapse_ts_index():
    with patch("synapse_vectorbt.adapter._load_rows", return_value=SYNTHETIC_ROWS):
        df = from_synapse(":memory:", "AAPL", 0, 9_999_999_999)

    assert isinstance(df.index, pd.DatetimeIndex)
    assert str(df.index.tz) == "UTC"
    assert df.index[0] == pd.Timestamp("2023-11-14 22:13:20", tz="UTC")
