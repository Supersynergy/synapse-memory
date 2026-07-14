//! In-memory int8-quantized brute-force index.
//!
//! Builds once from `(id, Vec<f32>)` pairs, stores per-row int8 codes + scales.
//! Every `search` call is a SIMD-parallel dot-product over the whole corpus —
//! for M4 Max we measure **325 µs / query @ 100 k × 384** (see
//! `docs/bench_2026-04-24/progression.md`).
//!
//! Recall ≥ 0.97 vs f32 ground truth (see integration test in synapse-core).
//!
//! # Example
//! ```
//! # #[cfg(feature = "simsimd")] {
//! use synapse_core::turbo::inmem_i8_index::InMemoryI8Index;
//! let rows = vec![
//!     (1_i64, vec![1.0_f32, 0.0, 0.0, 0.0]),
//!     (2,     vec![0.0_f32, 1.0, 0.0, 0.0]),
//! ];
//! let idx = InMemoryI8Index::build(rows);
//! let top = idx.search(&[1.0, 0.0, 0.0, 0.0], 1);
//! assert_eq!(top[0].0, 1);
//! # }
//! ```

#![allow(clippy::type_complexity)]

use rayon::prelude::*;

/// Single-thread threshold: corpora below this use plain iterators so that
/// concurrent Tokio tasks each get a full core rather than Rayon saturating all
/// cores per query and serializing multi-query throughput.
///
/// Above the threshold a dedicated Rayon pool (`SEARCH_POOL`) with 1 thread is
/// used — this keeps the hot path compiled as parallel Rayon while avoiding the
/// global pool contention that tanks QPS under 12-concurrent-query load.
const SINGLE_THREAD_THRESHOLD: usize = 500_000;

/// Minimum rows per rayon chunk — kept for the rescore path which operates on
/// small candidate sets and never hits the threshold.
const SEARCH_MIN_LEN: usize = 256;

/// Dedicated 1-thread Rayon pool: gives us `par_chunks` SIMD dispatch without
/// stealing cores from concurrent Tokio tasks.  One thread → one core per
/// query; concurrency = Tokio task count (already bounded by block_in_place).
static SEARCH_POOL: std::sync::LazyLock<rayon::ThreadPool> = std::sync::LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .thread_name(|i| format!("synapse-search-{i}"))
        .build()
        .expect("rayon pool build")
});

/// Dense int8-quantized brute-force index.
pub struct InMemoryI8Index {
    ids: Vec<i64>,
    codes: Vec<i8>,
    scales: Vec<f32>,
    dim: usize,
    /// Prebuilt id → row lookup; populated lazily on first `rescore()` call.
    id_to_row: std::sync::OnceLock<std::collections::HashMap<i64, usize>>,
}

impl InMemoryI8Index {
    /// Build from `(id, vec_f32)` pairs. Empty input yields a zero-dim index.
    ///
    /// # Panics
    /// Panics if rows are ragged (unequal dimensions).
    #[must_use]
    pub fn build(rows: Vec<(i64, Vec<f32>)>) -> Self {
        if rows.is_empty() {
            return Self {
                ids: Vec::new(),
                codes: Vec::new(),
                scales: Vec::new(),
                dim: 0,
                id_to_row: std::sync::OnceLock::new(),
            };
        }
        let dim = rows[0].1.len();
        assert!(rows.iter().all(|(_, v)| v.len() == dim), "ragged rows");

        let n = rows.len();
        let mut ids = Vec::with_capacity(n);
        let mut codes = vec![0_i8; n * dim];
        let mut scales = vec![0_f32; n];

        for (i, (id, vec)) in rows.into_iter().enumerate() {
            ids.push(id);
            let absmax = vec.iter().fold(0_f32, |a, &v| a.max(v.abs())).max(1e-8);
            scales[i] = absmax / 127.0;
            let inv = 1.0 / absmax;
            for (j, v) in vec.into_iter().enumerate() {
                codes[i * dim + j] = (v * inv * 127.0).round().clamp(-127.0, 127.0) as i8;
            }
        }
        Self {
            ids,
            codes,
            scales,
            dim,
            id_to_row: std::sync::OnceLock::new(),
        }
    }

    /// Number of indexed rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    /// Empty index probe.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    /// Dimensionality.
    #[must_use]
    pub const fn dim(&self) -> usize {
        self.dim
    }

    /// Rescore only a subset of ids — ideal as a rerank stage after a
    /// cheaper candidate-gen pass (Hamming, MRL, HNSW). Returns ids in
    /// score-descending order.
    ///
    /// Unknown ids are silently skipped. O(candidates × dim).
    pub fn rescore(&self, query: &[f32], candidate_ids: &[i64]) -> Vec<(i64, f32)> {
        if query.len() != self.dim || candidate_ids.is_empty() {
            return Vec::new();
        }
        let q_abs = query.iter().fold(0_f32, |a, &v| a.max(v.abs())).max(1e-8);
        let q_inv = 1.0 / q_abs;
        let q_scale = q_abs / 127.0;
        let q_codes: Vec<i8> = query
            .iter()
            .map(|v| (*v * q_inv * 127.0).round().clamp(-127.0, 127.0) as i8)
            .collect();

        // id → row index lookup — built once on first call, reused after.
        let id_to_row = self.id_to_row.get_or_init(|| {
            let mut m = std::collections::HashMap::with_capacity(self.ids.len());
            for (i, id) in self.ids.iter().enumerate() {
                m.insert(*id, i);
            }
            m
        });
        let rows: Vec<usize> = candidate_ids
            .iter()
            .filter_map(|id| id_to_row.get(id).copied())
            .collect();

        let mut out: Vec<(i64, f32)> = rows
            .par_iter()
            .with_min_len(SEARCH_MIN_LEN)
            .map(|&i| {
                let row = &self.codes[i * self.dim..(i + 1) * self.dim];
                let s = self.scales[i];
                (self.ids[i], dot_i8(&q_codes, row) * s * q_scale)
            })
            .collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// Search — returns `(id, cosine-like score)` pairs, sorted best-first.
    ///
    /// When the `simsimd` feature is enabled, uses NEON dot_i8 per row; else
    /// uses the scalar path (still rayon-parallel).
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        if self.is_empty() || query.len() != self.dim {
            return Vec::new();
        }
        // Quantize query with the same symmetric-per-row scheme.
        let q_abs = query.iter().fold(0_f32, |a, &v| a.max(v.abs())).max(1e-8);
        let q_inv = 1.0 / q_abs;
        let q_scale = q_abs / 127.0;
        let q_codes: Vec<i8> = query
            .iter()
            .map(|v| (*v * q_inv * 127.0).round().clamp(-127.0, 127.0) as i8)
            .collect();

        let scores: Vec<f32> = if self.codes.len() / self.dim >= SINGLE_THREAD_THRESHOLD {
            SEARCH_POOL.install(|| {
                self.codes
                    .par_chunks(self.dim)
                    .with_min_len(SEARCH_MIN_LEN)
                    .zip(self.scales.par_iter().with_min_len(SEARCH_MIN_LEN))
                    .map(|(row, &s)| dot_i8(&q_codes, row) * s * q_scale)
                    .collect()
            })
        } else {
            self.codes
                .chunks(self.dim)
                .zip(self.scales.iter())
                .map(|(row, &s)| dot_i8(&q_codes, row) * s * q_scale)
                .collect()
        };

        let k = k.min(scores.len());
        if k == 0 {
            return Vec::new();
        }
        let mut idx: Vec<usize> = (0..scores.len()).collect();
        idx.select_nth_unstable_by(k - 1, |a, b| {
            scores[*b]
                .partial_cmp(&scores[*a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        idx.truncate(k);
        idx.sort_by(|a, b| {
            scores[*b]
                .partial_cmp(&scores[*a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        idx.into_iter().map(|i| (self.ids[i], scores[i])).collect()
    }
}

#[cfg(feature = "simsimd")]
fn dot_i8(a: &[i8], b: &[i8]) -> f32 {
    crate::turbo::simsimd_kernels::dot_i8(a, b)
        .map(|v| v as f32)
        .unwrap_or(0.0)
}

#[cfg(not(feature = "simsimd"))]
fn dot_i8(a: &[i8], b: &[i8]) -> f32 {
    let mut sum = 0_i64;
    for (x, y) in a.iter().zip(b) {
        sum += i64::from(*x) * i64::from(*y);
    }
    sum as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
        v.into_iter().map(|x| x / n).collect()
    }

    #[test]
    fn exact_match_wins() {
        let rows = vec![
            (10_i64, unit(vec![1.0, 0.0, 0.0, 0.0])),
            (20, unit(vec![0.0, 1.0, 0.0, 0.0])),
            (30, unit(vec![0.0, 0.0, 1.0, 0.0])),
        ];
        let idx = InMemoryI8Index::build(rows);
        let top = idx.search(&unit(vec![1.0, 0.0, 0.0, 0.0]), 1);
        assert_eq!(top[0].0, 10);
    }

    #[test]
    fn empty_index_returns_empty() {
        let idx = InMemoryI8Index::build(Vec::new());
        assert!(idx.is_empty());
        assert!(idx.search(&[1.0], 5).is_empty());
    }

    #[test]
    fn dim_mismatch_returns_empty() {
        let rows = vec![(1_i64, unit(vec![1.0, 0.0, 0.0, 0.0]))];
        let idx = InMemoryI8Index::build(rows);
        assert!(idx.search(&[1.0, 0.0], 1).is_empty());
    }

    #[test]
    fn topk_larger_than_corpus_ok() {
        let rows = vec![(1_i64, unit(vec![1.0, 0.0])), (2, unit(vec![0.0, 1.0]))];
        let idx = InMemoryI8Index::build(rows);
        let top = idx.search(&unit(vec![1.0, 0.0]), 100);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, 1);
    }
}
