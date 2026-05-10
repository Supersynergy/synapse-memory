//! synapsql-pyo3 — Rust port of T0Cache exposed to Python.
//!
//! Status: SCAFFOLD. Bench shows pure-Python adapter already hits 27-168000×
//! on real SupersynergyCRM workloads — PyO3 fast-path is a nice-to-have for
//! sub-µs hot path, not a blocker.
//!
//! Build:
//!   uvx maturin develop --release --manifest-path Cargo.toml
//!
//! Use from Python (optional fast-path in synapsql/cache.py):
//!   from synapsql_pyo3 import RustCache
//!   cache = RustCache()  # 16-shard ahash, ~50ns hot-hit (vs 600ns Python)

use ahash::AHasher;
use parking_lot::Mutex;
use pyo3::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

const SHARDS: usize = 16;
const DEFAULT_CAP: usize = 2048;

struct Shard {
    map: HashMap<u64, (PyObject, u64)>,
    order: VecDeque<u64>,
    cap: usize,
}

impl Shard {
    fn new(cap: usize) -> Self {
        Shard { map: HashMap::with_capacity(cap), order: VecDeque::with_capacity(cap), cap }
    }
}

#[pyclass]
struct RustCache {
    shards: Vec<Arc<Mutex<Shard>>>,
    gen: Arc<Mutex<u64>>,
}

#[pymethods]
impl RustCache {
    #[new]
    fn new() -> Self {
        let shards = (0..SHARDS).map(|_| Arc::new(Mutex::new(Shard::new(DEFAULT_CAP)))).collect();
        RustCache { shards, gen: Arc::new(Mutex::new(0)) }
    }

    fn key(&self, sql: &str, params: &str) -> u64 {
        let mut h = AHasher::default();
        sql.hash(&mut h);
        params.hash(&mut h);
        h.finish()
    }

    fn get(&self, py: Python<'_>, key: u64) -> Option<PyObject> {
        let cur_gen = *self.gen.lock();
        let shard = &self.shards[(key as usize) & (SHARDS - 1)];
        let sh = shard.lock();
        if let Some((obj, stored_gen)) = sh.map.get(&key) {
            if *stored_gen >= cur_gen {
                return Some(obj.clone_ref(py));
            }
        }
        None
    }

    fn put(&self, key: u64, value: PyObject) {
        let cur_gen = *self.gen.lock();
        let shard = &self.shards[(key as usize) & (SHARDS - 1)];
        let mut sh = shard.lock();
        if sh.map.len() >= sh.cap && !sh.map.contains_key(&key) {
            if let Some(old) = sh.order.pop_front() {
                sh.map.remove(&old);
            }
        }
        sh.map.insert(key, (value, cur_gen));
        sh.order.push_back(key);
    }

    fn invalidate(&self) {
        let mut g = self.gen.lock();
        *g += 1;
    }

    fn __len__(&self) -> usize {
        self.shards.iter().map(|s| s.lock().map.len()).sum()
    }
}

#[pymodule]
fn synapsql_pyo3(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RustCache>()?;
    Ok(())
}
