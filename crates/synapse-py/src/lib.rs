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
use synapse_core::turbo::adaptive_router::{AdaptiveRouter, QueryHints, Strategy};
use synapse_core::turbo::inmem_hamming_index::InMemoryHammingIndex;
use synapse_core::turbo::inmem_i8_index::InMemoryI8Index;
use synapse_core::types::{PutRequest, SearchMode};
use synapse_core::Store;

/// Dense int8-quantized brute-force index, SIMSIMD-accelerated.
///
/// Typical usage:
///
/// ```python
/// import synapse
/// rows = [(doc_id, [0.1]*384), ...]
/// idx = synapse.I8Index.build(rows)
/// hits = idx.search([0.05]*384, k=10)   # [(id, score), ...]
/// ```
#[pyclass(name = "I8Index")]
pub struct PyI8Index {
    inner: InMemoryI8Index,
}

#[pymethods]
impl PyI8Index {
    /// Build from `(id, vec_f32)` pairs. Empty input yields an empty index.
    #[staticmethod]
    fn build(rows: Vec<(i64, Vec<f32>)>) -> PyResult<Self> {
        if rows.iter().any(|(_, v)| v.is_empty()) {
            return Err(PyValueError::new_err("empty vector in rows"));
        }
        let dim = rows.first().map(|r| r.1.len()).unwrap_or(0);
        if rows.iter().any(|(_, v)| v.len() != dim) {
            return Err(PyValueError::new_err("ragged rows"));
        }
        Ok(Self { inner: InMemoryI8Index::build(rows) })
    }

    #[pyo3(signature = (query, k=10))]
    fn search(&self, query: Vec<f32>, k: usize) -> PyResult<Vec<(i64, f32)>> {
        Ok(self.inner.search(&query, k))
    }

    fn len(&self) -> usize { self.inner.len() }
    fn is_empty(&self) -> bool { self.inner.is_empty() }
    fn dim(&self) -> usize { self.inner.dim() }
}

/// Dense 1-bit Hamming index — very fast candidate generation (~72% recall alone).
///
/// Pair with `I8Index.search_rerank` for full-recall sub-ms pipeline.
#[pyclass(name = "HammingIndex")]
pub struct PyHammingIndex {
    inner: InMemoryHammingIndex,
}

#[pymethods]
impl PyHammingIndex {
    #[staticmethod]
    fn build(rows: Vec<(i64, Vec<f32>)>) -> PyResult<Self> {
        let dim = rows.first().map(|r| r.1.len()).unwrap_or(0);
        if rows.iter().any(|(_, v)| v.len() != dim) {
            return Err(PyValueError::new_err("ragged rows"));
        }
        Ok(Self { inner: InMemoryHammingIndex::build(rows) })
    }

    #[pyo3(signature = (query, k=10))]
    fn search(&self, query: Vec<f32>, k: usize) -> PyResult<Vec<(i64, u32)>> {
        Ok(self.inner.search(&query, k))
    }

    fn len(&self) -> usize { self.inner.len() }
    fn is_empty(&self) -> bool { self.inner.is_empty() }
    fn dim(&self) -> usize { self.inner.dim() }
}

/// Two-stage candidate-gen + rerank pipeline.
///
/// Returns the top-k ids with full-recall int8 scoring after a wide
/// Hamming-based candidate pass. Typical settings: `candidates = 8 * k`.
#[pyfunction]
#[pyo3(signature = (hamming_idx, i8_idx, query, k=10, candidates=80))]
fn rerank(
    hamming_idx: &PyHammingIndex,
    i8_idx: &PyI8Index,
    query: Vec<f32>,
    k: usize,
    candidates: usize,
) -> PyResult<Vec<(i64, f32)>> {
    let cands = hamming_idx.inner.search(&query, candidates);
    if cands.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = cands.into_iter().map(|(id, _)| id).collect();
    let mut rescored = i8_idx.inner.rescore(&query, &ids);
    rescored.truncate(k);
    Ok(rescored)
}

/// Python-facing wrapper around the SIMSIMD / MRL adaptive strategy picker.
#[pyclass(name = "AdaptiveRouter")]
pub struct PyAdaptiveRouter {
    inner: Mutex<AdaptiveRouter>,
}

#[pymethods]
impl PyAdaptiveRouter {
    #[new]
    fn new() -> Self {
        Self { inner: Mutex::new(AdaptiveRouter::new()) }
    }

    /// Pick a strategy for the given query hints. Returns a short string
    /// identifier: `"scalar"`, `"rayon"`, `"simsimd_f32"`, `"simsimd_i8"`,
    /// `"simsimd_hamming"`, `"mrl_simsimd"`.
    #[pyo3(signature = (corpus_size, latency_budget_us=0, min_recall=0.0))]
    fn choose(&self, corpus_size: usize, latency_budget_us: u64, min_recall: f64) -> PyResult<&'static str> {
        let g = self.inner.lock().map_err(|_| PyRuntimeError::new_err("lock poisoned"))?;
        Ok(strategy_name(g.choose(&QueryHints { corpus_size, latency_budget_us, min_recall })))
    }

    /// Feed back observed wall-time (µs) + recall@10 (0..1).
    fn observe(&self, strategy: &str, us: f64, recall: f64) -> PyResult<()> {
        let s = strategy_from_name(strategy)
            .ok_or_else(|| PyValueError::new_err(format!("unknown strategy: {strategy}")))?;
        let mut g = self.inner.lock().map_err(|_| PyRuntimeError::new_err("lock poisoned"))?;
        g.observe(s, us, recall);
        Ok(())
    }

    /// Number of observations recorded so far.
    fn decisions(&self) -> PyResult<u64> {
        let g = self.inner.lock().map_err(|_| PyRuntimeError::new_err("lock poisoned"))?;
        Ok(g.decisions())
    }

    /// Current `(strategy, posterior_recall, ewma_us)` tuples.
    fn posterior(&self) -> PyResult<Vec<(&'static str, f64, f64)>> {
        let g = self.inner.lock().map_err(|_| PyRuntimeError::new_err("lock poisoned"))?;
        Ok(g.posterior_means()
            .into_iter()
            .map(|(s, r, u)| (strategy_name(s), r, u))
            .collect())
    }
}

const fn strategy_name(s: Strategy) -> &'static str {
    match s {
        Strategy::ScalarF32      => "scalar",
        Strategy::RayonF32       => "rayon",
        Strategy::SimSimdF32     => "simsimd_f32",
        Strategy::SimSimdI8      => "simsimd_i8",
        Strategy::SimSimdHamming => "simsimd_hamming",
        Strategy::MrlSimSimd     => "mrl_simsimd",
    }
}

fn strategy_from_name(s: &str) -> Option<Strategy> {
    Some(match s {
        "scalar"          => Strategy::ScalarF32,
        "rayon"           => Strategy::RayonF32,
        "simsimd_f32"     => Strategy::SimSimdF32,
        "simsimd_i8"      => Strategy::SimSimdI8,
        "simsimd_hamming" => Strategy::SimSimdHamming,
        "mrl_simsimd"     => Strategy::MrlSimSimd,
        _ => return None,
    })
}

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

    /// Vector search — client supplies the query embedding.
    ///
    /// Use this when embeddings are computed client-side (e.g. via a Python
    /// `sentence-transformers` or OpenAI call) to avoid a round-trip.
    #[pyo3(signature = (embedding, limit=10))]
    fn search_vec(&self, embedding: Vec<f32>, limit: usize) -> PyResult<Vec<(i64, String, f64)>> {
        let g = self.inner.lock().map_err(|_| PyRuntimeError::new_err("lock poisoned"))?;
        let hits = g
            .search("", SearchMode::Vec, Some(&embedding), limit)
            .map_err(|e| PyRuntimeError::new_err(format!("search_vec: {e}")))?;
        Ok(hits.into_iter().map(|h| (h.id, h.text, h.score)).collect())
    }

    /// Hybrid BM25 + vector search with RRF fusion.
    #[pyo3(signature = (q, embedding, limit=10))]
    fn search_hybrid(
        &self,
        q: &str,
        embedding: Vec<f32>,
        limit: usize,
    ) -> PyResult<Vec<(i64, String, f64)>> {
        let g = self.inner.lock().map_err(|_| PyRuntimeError::new_err("lock poisoned"))?;
        let hits = g
            .search(q, SearchMode::Hybrid, Some(&embedding), limit)
            .map_err(|e| PyRuntimeError::new_err(format!("search_hybrid: {e}")))?;
        Ok(hits.into_iter().map(|h| (h.id, h.text, h.score)).collect())
    }

    /// Insert a doc with a pre-computed embedding (client-side embedder).
    #[pyo3(signature = (text, embedding, uri=None, title=None))]
    fn put_with_embedding(
        &self,
        text: String,
        embedding: Vec<f32>,
        uri: Option<String>,
        title: Option<String>,
    ) -> PyResult<i64> {
        let req = PutRequest {
            uri,
            title,
            text,
            meta: None,
            embedding: Some(embedding),
        };
        let mut g = self.inner.lock().map_err(|_| PyRuntimeError::new_err("lock poisoned"))?;
        g.put(&req)
            .map_err(|e| PyRuntimeError::new_err(format!("put_with_embedding: {e}")))
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
    m.add_class::<PyAdaptiveRouter>()?;
    m.add_class::<PyI8Index>()?;
    m.add_class::<PyHammingIndex>()?;
    m.add_function(wrap_pyfunction!(rerank, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
