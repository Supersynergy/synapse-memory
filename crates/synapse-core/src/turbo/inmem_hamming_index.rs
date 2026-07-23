//! In-memory 1-bit Hamming index for candidate generation.
//!
//! Companion to [`InMemoryI8Index`]: binarize each vector to `ceil(dim/8)`
//! bytes (sign-bit), search with NEON popcount. 248 µs / query @ 100 k × 384
//! on M4 Max (see `docs/bench_2026-04-24/progression.md`).
//!
//! Recall alone is lossy (~72 %); pair with [`InMemoryI8Index`] rerank for
//! full-recall sub-ms search over 10 M rows.
//!
//! [`InMemoryI8Index`]: super::inmem_i8_index::InMemoryI8Index

#![allow(clippy::type_complexity)]

use rayon::prelude::*;

const HAMMING_MIN_LEN: usize = 512;
const SINGLE_THREAD_THRESHOLD: usize = 500_000;

static SEARCH_POOL: std::sync::LazyLock<rayon::ThreadPool> = std::sync::LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .thread_name(|i| format!("synapse-hamming-search-{i}"))
        .build()
        .expect("rayon hamming pool build")
});

/// 1-bit Hamming-distance brute-force index.
pub struct InMemoryHammingIndex {
    ids: Vec<i64>,
    bits: Vec<u8>, // row-major, bytes per row = bpr
    bpr: usize,
    dim: usize,
}

impl InMemoryHammingIndex {
    /// Build from `(id, vec_f32)` pairs. Each vector is sign-binarized.
    ///
    /// # Panics
    /// Panics on ragged rows.
    #[must_use]
    pub fn build(rows: Vec<(i64, Vec<f32>)>) -> Self {
        if rows.is_empty() {
            return Self {
                ids: Vec::new(),
                bits: Vec::new(),
                bpr: 0,
                dim: 0,
            };
        }
        let dim = rows[0].1.len();
        assert!(rows.iter().all(|(_, v)| v.len() == dim), "ragged rows");
        let bpr = dim.div_ceil(8);
        let n = rows.len();
        let mut ids = Vec::with_capacity(n);
        let mut bits = vec![0_u8; n * bpr];

        for (i, (id, vec)) in rows.into_iter().enumerate() {
            ids.push(id);
            for (j, v) in vec.into_iter().enumerate() {
                if v > 0.0 {
                    bits[i * bpr + j / 8] |= 1 << (j % 8);
                }
            }
        }
        Self {
            ids,
            bits,
            bpr,
            dim,
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

    /// Row IDs in insertion order. Exposed for cascade indexes that need to
    /// map result IDs back to code-index positions (e.g. RaBitQIndex).
    #[must_use]
    pub fn ids(&self) -> &[i64] {
        &self.ids
    }

    /// Position of `id` in the index, or None if absent.
    #[must_use]
    pub fn position_of(&self, id: i64) -> Option<usize> {
        self.ids.iter().position(|&x| x == id)
    }

    /// Top-k candidate ids by smallest Hamming distance.
    ///
    /// Bottleneck fix 2026-05-10: previously allocated `Vec<u32>` for ALL
    /// N rows (40MB heap at 10M rows). Now uses bounded max-heap of size k —
    /// O(k) memory, O(N log k) time. For typical k=10..100 this is 100k-1M×
    /// smaller heap footprint at 10M-vec scale.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(i64, u32)> {
        if self.is_empty() || query.len() != self.dim || k == 0 {
            return Vec::new();
        }
        let mut q_bits = vec![0_u8; self.bpr];
        for (j, v) in query.iter().enumerate() {
            if *v > 0.0 {
                q_bits[j / 8] |= 1 << (j % 8);
            }
        }
        let n = self.bits.len() / self.bpr;
        let k = k.min(n);

        // Bounded top-k via max-heap. Item = (dist, row_idx); BinaryHeap is max-heap
        // by default, so we keep the k smallest distances by evicting the max.
        use std::collections::BinaryHeap;

        let topk: Vec<(u32, usize)> = if n >= SINGLE_THREAD_THRESHOLD {
            SEARCH_POOL.install(|| {
                self.bits
                    .par_chunks(self.bpr)
                    .with_min_len(HAMMING_MIN_LEN)
                    .enumerate()
                    .fold(
                        || BinaryHeap::<(u32, usize)>::with_capacity(k + 1),
                        |mut heap, (i, row)| {
                            let d = hamming_u32(&q_bits, row);
                            if heap.len() < k {
                                heap.push((d, i));
                            } else if d < heap.peek().unwrap().0 {
                                heap.pop();
                                heap.push((d, i));
                            }
                            heap
                        },
                    )
                    .reduce(
                        || BinaryHeap::<(u32, usize)>::with_capacity(k + 1),
                        |mut a, b| {
                            for item in b.into_iter() {
                                if a.len() < k {
                                    a.push(item);
                                } else if item.0 < a.peek().unwrap().0 {
                                    a.pop();
                                    a.push(item);
                                }
                            }
                            a
                        },
                    )
                    .into_sorted_vec()
            })
        } else {
            let mut heap = BinaryHeap::<(u32, usize)>::with_capacity(k + 1);
            for (i, row) in self.bits.chunks(self.bpr).enumerate() {
                let d = hamming_u32(&q_bits, row);
                if heap.len() < k {
                    heap.push((d, i));
                } else if d < heap.peek().unwrap().0 {
                    heap.pop();
                    heap.push((d, i));
                }
            }
            heap.into_sorted_vec()
        };

        topk.into_iter().map(|(d, i)| (self.ids[i], d)).collect()
    }
}

#[cfg(feature = "simsimd")]
fn hamming_u32(q: &[u8], row: &[u8]) -> u32 {
    crate::turbo::simsimd_kernels::hamming_b8(q, row)
        .map(|f| f as u32)
        .unwrap_or(u32::MAX)
}

#[cfg(not(feature = "simsimd"))]
fn hamming_u32(q: &[u8], row: &[u8]) -> u32 {
    let mut acc = 0_u32;
    for (a, b) in q.iter().zip(row) {
        acc += (a ^ b).count_ones();
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vector_has_zero_distance() {
        let rows = vec![(1_i64, vec![1.0, -1.0, 0.5, -0.5])];
        let idx = InMemoryHammingIndex::build(rows);
        let r = idx.search(&[1.0, -1.0, 0.5, -0.5], 1);
        assert_eq!(r[0], (1, 0));
    }

    #[test]
    fn opposite_sign_is_all_bits() {
        let rows = vec![(1_i64, vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0])];
        let idx = InMemoryHammingIndex::build(rows);
        let r = idx.search(&[-1.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0], 1);
        assert_eq!(r[0], (1, 8));
    }

    #[test]
    fn empty_and_mismatch_safe() {
        let idx = InMemoryHammingIndex::build(Vec::new());
        assert!(idx.search(&[1.0], 5).is_empty());
        let idx2 = InMemoryHammingIndex::build(vec![(1, vec![1.0, -1.0])]);
        assert!(idx2.search(&[1.0], 1).is_empty()); // dim mismatch
    }
}
