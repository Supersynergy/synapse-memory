"""Tests for QlibSynapseHandler — no qlib or synapse-market-py required."""
from __future__ import annotations

from unittest.mock import patch, MagicMock
import pandas as pd
import pytest

from synapse_qlib.handler import QlibSynapseHandler, DEFAULT_FIELDS

TICKERS = ["AAPL", "MSFT"]

def _synthetic_rows(ticker: str, n: int = 3):
    base = {"AAPL": 100.0, "MSFT": 200.0}.get(ticker, 50.0)
    return [
        {"ts": 1_700_000_000 + i * 60, "open": base + i, "high": base + i + 1,
         "low": base + i - 1, "close": base + i + 0.5, "volume": 1000.0}
        for i in range(n)
    ]


def _make_handler(monkeypatch):
    def fake_load(self, ticker):
        rows = _synthetic_rows(ticker)
        import numpy as np
        ts = pd.to_datetime([r["ts"] for r in rows], unit="s", utc=True)
        data = {col: [r[col] for r in rows] for col in DEFAULT_FIELDS}
        return pd.DataFrame(data, index=ts)

    monkeypatch.setattr(QlibSynapseHandler, "_load_ticker", fake_load)
    return QlibSynapseHandler(":memory:", TICKERS)


def test_fetch_multiindex_shape(monkeypatch):
    h = _make_handler(monkeypatch)
    df = h.fetch()
    assert df.shape == (6, 5), f"Expected (6,5) got {df.shape}"
    assert df.index.names == ["instrument", "datetime"]


def test_fetch_instruments_present(monkeypatch):
    h = _make_handler(monkeypatch)
    df = h.fetch()
    instruments = df.index.get_level_values("instrument").unique().tolist()
    assert set(instruments) == {"AAPL", "MSFT"}
