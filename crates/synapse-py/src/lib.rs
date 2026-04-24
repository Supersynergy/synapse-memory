//! `synapse-py` — PyO3 bindings for synapse-core.
//!
//! Installs as `pip install synapse` (via maturin). Exposes the hot-path
//! kernels (SimSIMD + Matryoshka) and a thin wrapper around the agent-memory
//! `Store`. Designed to slot into existing Python stacks (LangChain,
//! LlamaIndex, Mem0) via downstream adapter packages.
//!
//! # Python example
//! ```python
//! import synapse
//! print(synapse.hamming_b8([0xAB]*16, [[0xAB]*16]))   # → [0]
//! print(synapse.truncate_row([1.0, 2.0, 3.0, 4.0], 2)) # → [~0.447, ~0.894]
//! ```

use std::sync::Mutex;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use synapse_core::types::{PutRequest, SearchMode};
use synapse_core::Store;

/// Cosine similarity between two equal-length f32 lists. Requires `simsimd` feature.
#[cfg(feature = "simsimd")]
#[pyfunction]
fn cos_f32(a: Vec<f32>, b: Vec<f32>) -> PyResult<f32> {
    synapse_core::turbo::simsimd_kernels::cos_f32(&a, &b)
        .ok_or_else(|| PyValueError::new_err("dim mismatch or nan"))
}

/// Int8 dot product between two equal-length i8 lists. Requires `simsimd` feature.
#[cfg(feature = "simsimd")]
#[pyfunction]
fn dot_i8(a: Vec<i8>, b: Vec<i8>) -> PyResult<i64> {
    synapse_core::turbo::simsimd_kernels::dot_i8(&a, &b)
        .ok_or_else(|| PyValueError::new_err("dim mismatch"))
}

/// Packed-bit 1-bit Hamming distance for a query against a batch of rows.
/// `db` is a list of equal-length byte vectors.
#[cfg(feature = "simsimd")]
#[pyfunction]
fn hamming_b8(q: Vec<u8>, db: Vec<Vec<u8>>) -> PyResult<Vec<f64>> {
    Ok(db.iter()
        .map(|row| synapse_core::turbo::simsimd_kernels::hamming_b8(&q, row).unwrap_or(f64::NAN))
        .collect())
}

/// Matryoshka truncation + L2 renormalize to the first `k` dims.
#[pyfunction]
fn truncate_row(v: Vec<f32>, k: usize) -> PyResult<Vec<f32>> {
    let out = synapse_core::matryoshka::truncate_row(&v, k);
    if out.is_empty() && k > 0 && k <= v.len() {
        return Err(PyValueError::new_err("truncate failed"));
    }
    Ok(out)
}

/// Python-facing thin wrapper around synapse-core `Store`.
///
/// Current surface:
/// * `Brain(path)` — open/create single-file store.
/// * `brain.put_text(text, uri=None, title=None)` — returns new doc id.
/// * `brain.search_lex(q, limit=10)` — FTS5 keyword search.
/// * `brain.count()` — document count via FTS probe.
///
/// Vector + hybrid search ship with the embedder bindings in v0.2.
#[pyclass(name = "Brain")]
pub struct PyBrain {
    inner: Mutex<Store>,
}

#[pymethods]
impl PyBrain {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let store = Store::open(path)
            .map_err(|e| PyRuntimeError::new_err(format!("store open: {e}")))?;
        Ok(Self { inner: Mutex::new(store) })
    }

    #[pyo3(signature = (text, uri=None, title=None))]
    fn put_text(&self, text: String, uri: Option<String>, title: Option<String>) -> PyResult<i64> {
        let req = PutRequest { uri, title, text, meta: None, embedding: None };
        let mut g = self.inner.lock().map_err(|_| PyRuntimeError::new_err("lock poisoned"))?;
        g.put(&req).map_err(|e| PyRuntimeError::new_err(format!("put: {e}")))
    }

    #[pyo3(signature = (q, limit=10))]
    fn search_lex(&self, q: &str, limit: usize) -> PyResult<Vec<(i64, String, f64)>> {
        let g = self.inner.lock().map_err(|_| PyRuntimeError::new_err("lock poisoned"))?;
        let hits = g
            .search(q, SearchMode::Lex, None, limit)
            .map_err(|e| PyRuntimeError::new_err(format!("search: {e}")))?;
        Ok(hits.into_iter().map(|h| (h.id, h.text, h.score)).collect())
    }
}

/// Module entry point. Exposed to Python as `synapse`.
#[pymodule]
fn synapse_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    #[cfg(feature = "simsimd")]
    {
        m.add_function(wrap_pyfunction!(cos_f32, m)?)?;
        m.add_function(wrap_pyfunction!(dot_i8, m)?)?;
        m.add_function(wrap_pyfunction!(hamming_b8, m)?)?;
    }
    m.add_function(wrap_pyfunction!(truncate_row, m)?)?;
    m.add_class::<PyBrain>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
