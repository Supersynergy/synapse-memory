use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::HashMap;

use synapse_market::Market;
use synapse_market::ffi::smx_query_range;

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
    fn append(&self, rows: Vec<(i64, f64, f64, f64, f64, f64)>) -> PyResult<()> {
        let m = unsafe { &*(self.market_ptr as *const Market) };
        m.ingest_ohlcv(&self.ticker, &rows).map_err(py_err)
    }

    /// Return OHLCV rows in [start, end] as list of dicts.
    fn range(&self, start: i64, end: i64) -> PyResult<Vec<HashMap<String, PyObject>>> {
        let m = unsafe { &*(self.market_ptr as *const Market) };
        let rows = smx_query_range(m, &self.ticker, start, end).map_err(py_err)?;
        Python::with_gil(|py| {
            let out = rows
                .iter()
                .map(|r| {
                    let mut d: HashMap<String, PyObject> = HashMap::new();
                    d.insert("ts".into(), r.0.into_py(py));
                    d.insert("open".into(), r.1.into_py(py));
                    d.insert("high".into(), r.2.into_py(py));
                    d.insert("low".into(), r.3.into_py(py));
                    d.insert("close".into(), r.4.into_py(py));
                    d.insert("volume".into(), r.5.into_py(py));
                    d
                })
                .collect();
            Ok(out)
        })
    }

    /// Return close prices as bytes (f64 LE) — zero-copy into numpy via frombuffer.
    fn closes_bytes(&self, start: i64, end: i64) -> PyResult<PyObject> {
        let m = unsafe { &*(self.market_ptr as *const Market) };
        let rows = smx_query_range(m, &self.ticker, start, end).map_err(py_err)?;
        let bytes: Vec<u8> = rows
            .iter()
            .flat_map(|r| r.4.to_le_bytes())
            .collect();
        Python::with_gil(|py| Ok(pyo3::types::PyBytes::new_bound(py, &bytes).into()))
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

#[pymodule]
fn synapse_market_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMarket>()?;
    m.add_class::<SeriesHandle>()?;
    Ok(())
}
