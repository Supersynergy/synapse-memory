"""Synapse-X DataHandler for Microsoft Qlib.

Stub-compatible: if pyqlib is not installed the class still works as a
plain data-fetcher (DataHandlerLP base is shimmed).
"""
from __future__ import annotations

from typing import List, Optional
import pandas as pd
import numpy as np

# ---------------------------------------------------------------------------
# Graceful shim — qlib is a heavy install; keep the adapter importable without it
# ---------------------------------------------------------------------------
try:
    from qlib.data.dataset.handler import DataHandlerLP as _Base  # type: ignore[import]
    _QLIB_AVAILABLE = True
except ImportError:
    class _Base:  # type: ignore[no-redef]
        def __init__(self, *args, **kwargs):
            pass
    _QLIB_AVAILABLE = False


DEFAULT_FIELDS = ["open", "high", "low", "close", "volume"]


class QlibSynapseHandler(_Base):
    """Qlib DataHandlerLP backed by a Synapse Market file.

    Parameters
    ----------
    market_path : str
        Path to .smx market file.
    instruments : list[str]
        List of ticker symbols to load.
    start_time : int
        Unix timestamp start (seconds).
    end_time : int
        Unix timestamp end (seconds).
    fields : list[str], optional
        Subset of OHLCV columns. Defaults to all five.
    """

    def __init__(
        self,
        market_path: str,
        instruments: List[str],
        start_time: int = 0,
        end_time: int = 9_999_999_999,
        fields: Optional[List[str]] = None,
    ):
        self.market_path = market_path
        self.instruments = instruments
        self.start_time = start_time
        self.end_time = end_time
        self.fields = fields or DEFAULT_FIELDS

        if _QLIB_AVAILABLE:
            super().__init__()

    def _load_ticker(self, ticker: str) -> pd.DataFrame:
        try:
            from synapse_market import PyMarket  # type: ignore[import]
            m = PyMarket.open(self.market_path)
            rows = m.series(ticker).range(self.start_time, self.end_time)
        except ModuleNotFoundError as exc:
            raise ImportError(
                "synapse-market-py not installed. "
                "Run: cd crates/synapse-market-py && maturin develop --release"
            ) from exc

        if not rows:
            return pd.DataFrame(columns=self.fields)

        ts = pd.to_datetime([r["ts"] for r in rows], unit="s", utc=True)
        data = {col: np.array([r[col] for r in rows], dtype=np.float64) for col in self.fields}
        return pd.DataFrame(data, index=ts)[self.fields]

    def fetch(
        self,
        selector: Optional[slice] = None,
        level: str = "datetime",
        col_set: str = "feature",
        squeeze: bool = False,
    ) -> pd.DataFrame:
        """Return multi-index (instrument, datetime) DataFrame.

        Compatible with qlib DataHandlerLP.fetch() signature.
        """
        frames = {}
        for ticker in self.instruments:
            frames[ticker] = self._load_ticker(ticker)

        if not frames:
            idx = pd.MultiIndex.from_tuples([], names=["instrument", "datetime"])
            return pd.DataFrame(columns=self.fields, index=idx)

        combined = pd.concat(frames, names=["instrument", "datetime"])
        if selector is not None:
            combined = combined.loc[selector]
        return combined
