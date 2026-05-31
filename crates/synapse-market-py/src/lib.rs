use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::collections::HashMap;

use synapse_market::Market;
use synapse_market::ffi::smx_query_range;

type OhlcvTuple = (i64, f64, f64, f64, f64, f64);
type PyRow = HashMap<String, PyObject>;
type PyRows = Vec<PyRow>;

fn py_err(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

#[pyclass]
struct SeriesHandle {
    ticker: String,
    market_ptr: usize, // raw ptr as usize — safe because Market outlives SeriesHandle
}

#[pymethods]
impl SeriesHandle {
    /// Append rows: list of (ts_secs, open, high, low, close, volume) tuples.
    fn append(&self, rows: Vec<OhlcvTuple>) -> PyResult<()> {
        let m = unsafe { &*(self.market_ptr as *const Market) };
        m.ingest_ohlcv(&self.ticker, &rows).map_err(py_err)
    }

    /// Return OHLCV rows in [start, end] as list of dicts.
    fn range(&self, start: i64, end: i64) -> PyResult<PyRows> {
        let m = unsafe { &*(self.market_ptr as *const Market) };
        let rows = smx_query_range(m, &self.ticker, start, end).map_err(py_err)?;
        Python::with_gil(|py| {
            let out = rows
                .iter()
                .map(|r| {
                    let mut d: HashMap<String, PyObject> = HashMap::new();
                    d.insert("ts".into(), r.0.into_pyobject(py)?.into_any().unbind());
                    d.insert("open".into(), r.1.into_pyobject(py)?.into_any().unbind());
                    d.insert("high".into(), r.2.into_pyobject(py)?.into_any().unbind());
                    d.insert("low".into(), r.3.into_pyobject(py)?.into_any().unbind());
                    d.insert("close".into(), r.4.into_pyobject(py)?.into_any().unbind());
                    d.insert("volume".into(), r.5.into_pyobject(py)?.into_any().unbind());
                    Ok(d)
                })
                .collect::<PyResult<_>>()?;
            Ok(out)
        })
    }

    /// Return close prices as bytes (f64 LE) — zero-copy into numpy via frombuffer.
    fn closes_bytes(&self, start: i64, end: i64) -> PyResult<PyObject> {
        let m = unsafe { &*(self.market_ptr as *const Market) };
        let rows = smx_query_range(m, &self.ticker, start, end).map_err(py_err)?;
        let bytes: Vec<u8> = rows.iter().flat_map(|r| r.4.to_le_bytes()).collect();
        Python::with_gil(|py| Ok(PyBytes::new(py, &bytes).into_any().unbind()))
    }
}

#[pyclass(unsendable)]
struct PyMarket {
    inner: Market,
}

#[pymethods]
impl PyMarket {
    /// Open (or create) market DB at path. Use ":memory:" for ephemeral.
    #[staticmethod]
    fn open(path: &str) -> PyResult<Self> {
        let m = if path == ":memory:" {
            Market::open_in_memory().map_err(py_err)?
        } else {
            Market::open(path).map_err(py_err)?
        };
        Ok(PyMarket { inner: m })
    }

    /// Get a SeriesHandle for ticker.
    fn series(&self, ticker: &str) -> SeriesHandle {
        SeriesHandle {
            ticker: ticker.to_uppercase(),
            market_ptr: &self.inner as *const Market as usize,
        }
    }
}

#[pymodule(name = "synapse_market")]
fn synapse_market_init(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMarket>()?;
    m.add_class::<SeriesHandle>()?;
    Ok(())
}
