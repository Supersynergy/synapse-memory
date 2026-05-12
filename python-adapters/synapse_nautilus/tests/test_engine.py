"""Tests for SynapseDataClient — no nautilus-trader or synapse-market-py required."""
from __future__ import annotations

from unittest.mock import patch
import pytest

from synapse_nautilus.engine import SynapseDataClient, SynapseBar

SYNTHETIC_ROWS = [
    {"ts": 1_700_000_000 + i * 60, "open": 100.0 + i, "high": 101.0 + i,
     "low": 99.0 + i, "close": 100.5 + i, "volume": 500.0}
    for i in range(4)
]


def _make_client(monkeypatch, tickers=("AAPL",)):
    monkeypatch.setattr(
        SynapseDataClient, "_iter_rows",
        lambda self, ticker: SYNTHETIC_ROWS,
    )
    return SynapseDataClient(":memory:", list(tickers))


def test_replay_emits_synapse_bars(monkeypatch):
    client = _make_client(monkeypatch)
    bars = client.replay()
    assert len(bars) == 4
    assert all(isinstance(b, SynapseBar) for b in bars)


def test_replay_event_types_correct(monkeypatch):
    client = _make_client(monkeypatch, tickers=("AAPL", "MSFT"))
    bars = client.replay()
    assert len(bars) == 8
    ids = {b.instrument_id for b in bars}
    assert ids == {"AAPL", "MSFT"}
    assert all(hasattr(b, "close") for b in bars)
