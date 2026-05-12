# Migration Guide

## Coming from SQLite?

synapse-market stores OHLCV in columnar mmap pages — no SQL dialect, no schema migrations.

| SQLite concept | synapse-market equivalent |
|---|---|
| `CREATE TABLE ohlcv (ticker TEXT, ts INTEGER, o REAL, h REAL, l REAL, c REAL, v REAL)` | `Market::open(path)` — schema implicit |
| `INSERT INTO ohlcv VALUES (...)` | `series.append(&[Bar { ts, o, h, l, c, v }])` |
| `SELECT * FROM ohlcv WHERE ticker=? AND ts BETWEEN ? AND ?` | `series.range(start, end)` |
| `SELECT AVG(c) FROM ohlcv WHERE ticker=?` | `analytics::mean_close(&series, start, end)` |
| `CREATE INDEX idx ON ohlcv(ticker, ts)` | implicit — pages are delta-encoded, sequential by ts |
| SQLite file (`.db`) | synapse-market store dir (`.smx/`) |

**When to stay on SQLite**: < 1k bars/ticker, ad-hoc queries, shared multi-writer access. synapse-market wins at ≥1k bars on repeated hot-path reads.

---

## Coming from kdb+/q?

| q symbol | synapse-market equivalent |
|---|---|
| `` `AAPL `` | `"AAPL"` (UTF-8 ticker string) |
| `t: ([] time:...; price:...)` keyed table | `series.append(&[Bar {...}])` columnar page |
| `select from t where time within (t1;t2)` | `series.range(t1, t2)` |
| `avg price` | `analytics::mean_close(...)` |
| `cor[x;y]` | `analytics::pearson_corr_matrix(tickers, days)` — AMX-accelerated 117× on M4 Max |
| `.Q.dpft` partitioned HDB | `Market::open(path)` — single file, no partition management |
| IPC port (`:5010`) | C ABI (`smx_series_range_close`) or MCP tools |

**Not yet implemented**: time-series joins (aj/wj), multi-key tables, q IPC protocol. Use the C ABI or Python bindings for complex joins.

---

## Coming from Parquet / Arrow?

| Parquet/Arrow pattern | synapse-market equivalent |
|---|---|
| `pd.read_parquet("ohlcv.parquet")` | `PyMarket.open(path).series("AAPL").range(start, end)` |
| `pq.write_table(table, "ohlcv.parquet")` | `series.append(rows)` |
| `pyarrow.RecordBatch` | `series.closes_bytes(start, end)` → `np.frombuffer(raw, np.float64)` |
| Column scan across all tickers | not yet (Arrow IPC zero-copy export deferred to W8) |
| Predicate pushdown | implicit — range query only reads relevant pages |

**Ingest path from existing Parquet**:

```python
import pyarrow.parquet as pq
from synapse_market import PyMarket

m = PyMarket.open("/tmp/market.db")
table = pq.read_table("ohlcv.parquet")  # columns: ticker, ts, open, high, low, close, volume

for ticker, group in table.group_by("ticker"):
    s = m.series(ticker.as_py())
    rows = [
        (row["ts"], row["open"], row["high"], row["low"], row["close"], row["volume"])
        for row in group.to_pylist()
    ]
    s.append(rows)
```

**Arrow IPC zero-copy** (planned W8): `series.to_arrow_ipc(start, end)` → `pyarrow.ipc.open_stream(bytes)` — no copy for downstream pandas/polars.
