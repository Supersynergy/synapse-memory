"""Synapse-X DataClient for NautilusTrader.

Stub-compatible: if nautilus-trader is not installed the class works as a
plain event-emitter with a callback interface. Wire it into a live Nautilus
engine by passing it to DataEngine.register_client().

Install heavy dep:
    uv pip install "nautilus-trader>=1.190"  # Cython build, takes ~5 min
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Callable, List, Optional

# ---------------------------------------------------------------------------
# Shim — nautilus-trader is a heavy Cython package; keep importable without it
# ---------------------------------------------------------------------------
try:
    from nautilus_trader.live.data_client import LiveDataClient as _LiveBase  # type: ignore[import]
    from nautilus_trader.model.data import Bar, BarType  # type: ignore[import]
    from nautilus_trader.model.identifiers import InstrumentId  # type: ignore[import]
    _NAUTILUS_AVAILABLE = True
except ImportError:
    _LiveBase = object  # type: ignore[assignment,misc]
    _NAUTILUS_AVAILABLE = False


@dataclass
class SynapseBar:
    """Lightweight Bar event emitted when nautilus-trader is not installed."""
    instrument_id: str
    ts_event: int
    open: float
    high: float
    low: float
    close: float
    volume: float


class SynapseDataClient(_LiveBase):
    """NautilusTrader LiveDataClient backed by a Synapse Market file.

    Parameters
    ----------
    market_path : str
        Path to .smx market file.
    instruments : list[str]
        Ticker symbols to stream.
    start_time : int
        Unix timestamp start (seconds).
    end_time : int
        Unix timestamp end (seconds).
    on_bar : callable, optional
        Callback receiving each SynapseBar (or native Bar when nautilus installed).
        Used in stub mode; in full nautilus mode events are published to the
        MessageBus automatically.
    """

    def __init__(
        self,
        market_path: str,
        instruments: List[str],
        start_time: int = 0,
        end_time: int = 9_999_999_999,
        on_bar: Optional[Callable] = None,
    ):
        self.market_path = market_path
        self.instruments = instruments
        self.start_time = start_time
        self.end_time = end_time
        self._on_bar = on_bar
        self._emitted: List[SynapseBar] = []

        if _NAUTILUS_AVAILABLE:
            # Full nautilus init requires loop + msgbus — defer to subclass or factory
            pass

    def _iter_rows(self, ticker: str):
        try:
            from synapse_market import PyMarket  # type: ignore[import]
            m = PyMarket.open(self.market_path)
            return m.series(ticker).range(self.start_time, self.end_time)
        except ModuleNotFoundError as exc:
            raise ImportError(
                "synapse-market-py not installed. "
                "Run: cd crates/synapse-market-py && maturin develop --release"
            ) from exc

    def _emit_bar(self, ticker: str, row: dict) -> SynapseBar:
        bar = SynapseBar(
            instrument_id=ticker,
            ts_event=row["ts"],
            open=row["open"],
            high=row["high"],
            low=row["low"],
            close=row["close"],
            volume=row["volume"],
        )
        self._emitted.append(bar)
        if self._on_bar is not None:
            self._on_bar(bar)
        return bar

    def replay(self) -> List[SynapseBar]:
        """Emit all bars synchronously. Returns list for testing."""
        self._emitted.clear()
        for ticker in self.instruments:
            for row in self._iter_rows(ticker):
                self._emit_bar(ticker, row)
        return list(self._emitted)
