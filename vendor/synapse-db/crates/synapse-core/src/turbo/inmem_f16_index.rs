//! In-memory f16-storage brute-force index.
//!
//! Pairs 50 % RAM savings (vs fp32) with full-fp32 compute on the hot path.
//! Useful when corpus fits in RAM at f16 but wouldn't at fp32, or when many
//! indices share L2 cache.
//!
//! Recall vs fp32 ground truth: ≥ 0.99 on normalized embeddings.

#![allow(clippy::type_complexity)]

use rayon::prelude::*;

#[cfg(not(feature = "simsimd"))]
use crate::turbo::f16_kernels::{cos_f16_row, pack_f16_rows};
#[cfg(feature = "simsimd")]
use crate::turbo::f16_kernels::{cos_f16_row_prepared, pack_f16_rows, prepare_query_f16};

const SEARCH_MIN_LEN: usize = 256;
const SINGLE_THREAD_THRESHOLD: usize = 500_000;

static SEARCH_POOL: std::sync::LazyLock<rayon::ThreadPool> = std::sync::LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .thread_name(|i| format!("synapse-f16-search-{i}"))
        .build()
        .expect("rayon f16 pool build")
});

/// Dense f16-stored brute-force cosine index.
pub struct InMemoryF16Index {
    ids: Vec<i64>,
    /// Row-major packed f16: `rows * dim * 2` bytes.
    packed: Vec<u8>,
    dim: usize,
    /// bytes per row (`dim * 2`)
    bpr: usize,
}

impl InMemoryF16Index {
    /// Build from `(id, vec_f32)` pairs.
    ///
    /// # Panics
    /// Panics on ragged rows.
    #[must_use]
    pub fn build(rows: Vec<(i64, Vec<f32>)>) -> Self {
        if rows.is_empty() {
            return Self {
                ids: Vec::new(),
                packed: Vec::new(),
                dim: 0,
                bpr: 0,
            };
        }
        let dim = rows[0].1.len();
        assert!(rows.iter().all(|(_, v)| v.len() == dim), "ragged rows");
        let ids: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
        let vecs: Vec<Vec<f32>> = rows.into_iter().map(|(_, v)| v).collect();
        let packed = pack_f16_rows(&vecs);
        Self {
            ids,
            packed,
            dim,
            bpr: dim * 2,
        }
    }

    /// Row count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    /// Empty probe.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    /// Dim.
    #[must_use]
    pub const fn dim(&self) -> usize {
        self.dim
    }
    /// Bytes stored in the index (excludes id vec).
    #[must_use]
    pub fn packed_bytes(&self) -> usize {
        self.packed.len()
    }

    /// Search top-k. Returns `(id, cosine score)`, sorted best-first.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        if self.is_empty() || query.len() != self.dim {
            return Vec::new();
        }
        // Bottleneck fix 2026-05-10: hoist query f16 conversion out of par_chunks loop.
        // Before: cos_f16_row allocated query Vec on every row → N allocs per search.
        // After: prepare_query_f16 once, cos_f16_row_prepared per row → 1 alloc per search.
        #[cfg(feature = "simsimd")]
        let q_prepared = prepare_query_f16(query);

        let scores: Vec<f32> = if self.packed.len() / self.bpr >= SINGLE_THREAD_THRESHOLD {
            SEARCH_POOL.install(|| {
                self.packed
                    .par_chunks(self.bpr)
                    .with_min_len(SEARCH_MIN_LEN)
                    .map(|row| {
                        #[cfg(feature = "simsimd")]
                        {
                            cos_f16_row_prepared(&q_prepared, row).unwrap_or(0.0)
                        }
                        #[cfg(not(feature = "simsimd"))]
                        {
                            cos_f16_row(query, row).unwrap_or(0.0)
                        }
                    })
                    .collect()
            })
        } else {
            self.packed
                .chunks(self.bpr)
                .map(|row| {
                    #[cfg(feature = "simsimd")]
                    {
                        cos_f16_row_prepared(&q_prepared, row).unwrap_or(0.0)
                    }
                    #[cfg(not(feature = "simsimd"))]
                    {
                        cos_f16_row(query, row).unwrap_or(0.0)
                    }
                })
                .collect()
        };
        let k = k.min(scores.len());
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
            (1_i64, unit(vec![1.0, 0.0, 0.0, 0.0])),
            (2, unit(vec![0.0, 1.0, 0.0, 0.0])),
        ];
        let idx = InMemoryF16Index::build(rows);
        let hits = idx.search(&unit(vec![1.0, 0.0, 0.0, 0.0]), 1);
        assert_eq!(hits[0].0, 1);
    }

    #[test]
    fn packed_bytes_is_half_of_fp32() {
        let rows: Vec<(i64, Vec<f32>)> = (0..10).map(|i| (i as i64, vec![0.1_f32; 128])).collect();
        let idx = InMemoryF16Index::build(rows);
        // 10 rows × 128 dim × 2 bytes = 2560
        assert_eq!(idx.packed_bytes(), 2560);
        // vs fp32: 10 × 128 × 4 = 5120 → 50% reduction
    }

    #[test]
    fn empty_and_mismatch_safe() {
        let idx = InMemoryF16Index::build(Vec::new());
        assert!(idx.search(&[1.0], 5).is_empty());
        let idx2 = InMemoryF16Index::build(vec![(1, vec![1.0, 0.0])]);
        assert!(idx2.search(&[1.0], 1).is_empty());
    }
}
