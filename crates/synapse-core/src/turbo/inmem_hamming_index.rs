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

use rayon::prelude::*;

/// 1-bit Hamming-distance brute-force index.
pub struct InMemoryHammingIndex {
    ids: Vec<i64>,
    bits: Vec<u8>,   // row-major, bytes per row = bpr
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
            return Self { ids: Vec::new(), bits: Vec::new(), bpr: 0, dim: 0 };
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
        Self { ids, bits, bpr, dim }
    }

    /// Row count.
    #[must_use]
    pub fn len(&self) -> usize { self.ids.len() }
    /// Empty probe.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.ids.is_empty() }
    /// Dim.
    #[must_use]
    pub const fn dim(&self) -> usize { self.dim }

    /// Top-k candidate ids by smallest Hamming distance.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(i64, u32)> {
        if self.is_empty() || query.len() != self.dim {
            return Vec::new();
        }
        let mut q_bits = vec![0_u8; self.bpr];
        for (j, v) in query.iter().enumerate() {
            if *v > 0.0 {
                q_bits[j / 8] |= 1 << (j % 8);
            }
        }
        let dists: Vec<u32> = self
            .bits
            .par_chunks(self.bpr)
            .map(|row| hamming_u32(&q_bits, row))
            .collect();
        let k = k.min(dists.len());
        let mut idx: Vec<usize> = (0..dists.len()).collect();
        idx.select_nth_unstable_by(k - 1, |a, b| dists[*a].cmp(&dists[*b]));
        idx.truncate(k);
        idx.sort_by(|a, b| dists[*a].cmp(&dists[*b]));
        idx.into_iter().map(|i| (self.ids[i], dists[i])).collect()
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
