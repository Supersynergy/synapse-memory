"""
Smoke-tests for synapse-market Python bindings.
Run after: maturin develop (from crates/synapse-market-py/)
"""
import pytest
import struct

# Module is installed by maturin develop
import synapse_market


ROWS = [
    (1_700_000_000, 100.0, 105.0, 99.0, 103.0, 1_000.0),
    (1_700_086_400, 103.0, 110.0, 102.0, 108.0, 1_200.0),
    (1_700_172_800, 108.0, 112.0, 107.0, 111.0, 900.0),
]


def test_open_in_memory():
    m = synapse_market.PyMarket.open(":memory:")
    assert m is not None


def test_series_append_and_range():
    m = synapse_market.PyMarket.open(":memory:")
    s = m.series("AAPL")
    s.append(ROWS)
    result = s.range(1_700_000_000, 1_700_172_800)
    assert len(result) == 3
    assert result[0]["close"] == pytest.approx(103.0)


def test_series_range_empty():
    m = synapse_market.PyMarket.open(":memory:")
    s = m.series("MSFT")
    result = s.range(0, 1)
    assert result == []


def test_closes_bytes():
    m = synapse_market.PyMarket.open(":memory:")
    s = m.series("TSLA")
    s.append(ROWS)
    raw = s.closes_bytes(1_700_000_000, 1_700_172_800)
    closes = struct.unpack(f"{len(raw)//8}d", raw)
    assert len(closes) == 3
    assert closes[1] == pytest.approx(108.0)


def test_multiple_tickers_isolated():
    m = synapse_market.PyMarket.open(":memory:")
    m.series("SPY").append(ROWS)
    m.series("QQQ").append(ROWS[:1])
    assert len(m.series("SPY").range(0, 2_000_000_000)) == 3
    assert len(m.series("QQQ").range(0, 2_000_000_000)) == 1
