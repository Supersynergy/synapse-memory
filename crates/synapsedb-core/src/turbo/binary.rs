//! Binary Quantization — 32× compression for ultra-fast pre-filtering.
//!
//! Each f32 dimension → 1 bit (sign bit). 384-dim f32 (1536 bytes) → 48 bytes.
//! Hamming distance as coarse similarity (XOR + POPCNT).
//!
//! Pattern validated from:
//! - ruvnet/RuVector `binary.rs`: `hamming_distance()` with POPCNT
//! - lance-format/lance `bq/builder.rs`: `RabitQuantizer` binary codes
//!
//! Usage: Pre-filter with binary quantization, then refine with full f32/i8.

use super::simd;

/// Binary quantization index — 1 bit per dimension.
pub struct BinaryIndex {
    /// Packed bits: ceil(dim/8) bytes per vector, row-major
    codes: Vec<u8>,
    ids: Vec<i64>,
    n_vectors: usize,
    dim: usize,
    /// Bytes per vector (ceil(dim/8))
    code_size: usize,
    /// Original f32 data for refinement (optional, kept for two-phase search)
    f32_data: Option<Vec<f32>>,
}

impl BinaryIndex {
    /// Build binary index from f32 matrix (pre-normalized).
    /// If `keep_f32` is true, retains original data for refinement phase.
    pub fn from_matrix(
        matrix: &ndarray::Array2<f32>,
        ids: &[i64],
        keep_f32: bool,
    ) -> Self {
        let n_vectors = matrix.nrows();
        let dim = matrix.ncols();
        let code_size = (dim + 7) / 8;

        let mut codes = Vec::with_capacity(n_vectors * code_size);
        for row in matrix.rows() {
            codes.extend_from_slice(&binarize(row.as_slice().unwrap()));
        }

        let f32_data = if keep_f32 {
            Some(
                matrix
                    .as_slice()
                    .map(|s| s.to_vec())
                    .unwrap_or_else(|| {
                        let mut v = Vec::with_capacity(n_vectors * dim);
                        for row in matrix.rows() {
                            v.extend_from_slice(row.as_slice().unwrap());
                        }
                        v
                    }),
            )
        } else {
            None
        };

        Self {
            codes,
            ids: ids.to_vec(),
            n_vectors,
            dim,
            code_size,
            f32_data,
        }
    }

    /// Binary-only search using Hamming distance.
    /// Fast but low precision — use for pre-filtering.
    pub fn search_binary(&self, query: &[f32], k: usize) -> Vec<(i64, u32)> {
        if query.len() != self.dim || self.n_vectors == 0 {
            return Vec::new();
        }

        let q_binary = binarize(query);
        let k = k.min(self.n_vectors);
        if k == 0 {
            return Vec::new();
        }

        // Compute Hamming distances
        let distances: Vec<u32> = (0..self.n_vectors)
            .map(|i| {
                let offset = i * self.code_size;
                hamming_distance(
                    &self.codes[offset..offset + self.code_size],
                    &q_binary,
                )
            })
            .collect();

        // Partial sort for top-k (lowest Hamming = most similar)
        let mut indices: Vec<usize> = (0..self.n_vectors).collect();
        indices.select_nth_unstable_by(k - 1, |&a, &b| distances[a].cmp(&distances[b]));

        let mut results: Vec<(i64, u32)> = indices[..k]
            .iter()
            .map(|&i| (self.ids[i], distances[i]))
            .collect();
        results.sort_by_key(|&(_, d)| d);
        results
    }

    /// Two-phase search: binary pre-filter → f32 refinement.
    /// Requires `keep_f32=true` at construction.
    pub fn search_twophase(&self, query: &[f32], k: usize, prefilter_factor: usize) -> Vec<(i64, f32)> {
        let f32_data = match &self.f32_data {
            Some(d) => d,
            None => return Vec::new(),
        };

        if query.len() != self.dim || self.n_vectors == 0 {
            return Vec::new();
        }

        let q_norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        if q_norm < 1e-10 {
            return Vec::new();
        }
        let q_normalized: Vec<f32> = query.iter().map(|x| x / q_norm).collect();

        let k = k.min(self.n_vectors);
        if k == 0 {
            return Vec::new();
        }

        // Phase 1: Binary pre-filter for top candidates
        let prefilter_k = (k * prefilter_factor).min(self.n_vectors);
        let candidates = self.search_binary(query, prefilter_k);

        // Phase 2: Refine with f32 dot product
        let mut refined: Vec<(i64, f32)> = candidates
            .iter()
            .map(|&(id, _)| {
                // Find index from id
                let idx = self.ids.iter().position(|&x| x == id).unwrap_or(0);
                let offset = idx * self.dim;
                let row = &f32_data[offset..offset + self.dim];
                let sim = simd::dot_f32(row, &q_normalized);
                (id, 1.0 - sim)
            })
            .collect();

        if refined.len() > k {
            refined.select_nth_unstable_by(k - 1, |a, b| {
                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
            });
            refined.truncate(k);
        }
        refined.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        refined
    }

    pub fn len(&self) -> usize {
        self.n_vectors
    }

    pub fn is_empty(&self) -> bool {
        self.n_vectors == 0
    }

    /// Memory usage for binary codes only (excluding f32 data).
    pub fn binary_memory_bytes(&self) -> usize {
        self.codes.len() + self.ids.len() * 8
    }

    /// Compression ratio vs f32.
    pub fn compression_ratio(&self) -> f32 {
        let f32_bytes = self.n_vectors * self.dim * 4;
        if f32_bytes == 0 {
            return 1.0;
        }
        f32_bytes as f32 / self.codes.len() as f32
    }
}

/// Binarize a f32 vector: 1 bit per dimension (sign bit).
/// Positive → 1, negative/zero → 0.
fn binarize(v: &[f32]) -> Vec<u8> {
    let code_size = (v.len() + 7) / 8;
    let mut code = vec![0u8; code_size];
    for (i, &val) in v.iter().enumerate() {
        if val > 0.0 {
            code[i / 8] |= 1 << (i % 8);
        }
    }
    code
}

/// Hamming distance between two binary codes (XOR + POPCNT).
#[inline]
fn hamming_distance(a: &[u8], b: &[u8]) -> u32 {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| (x ^ y).count_ones())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    fn make_normalized_matrix(n: usize, dim: usize) -> (Array2<f32>, Vec<i64>) {
        let mut data = Vec::with_capacity(n * dim);
        for i in 0..n {
            let mut row = Vec::with_capacity(dim);
            for j in 0..dim {
                row.push(((i * dim + j) as f32).sin() * 0.1);
            }
            let norm: f32 = row.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 1e-10 {
                for x in &mut row {
                    *x /= norm;
                }
            }
            data.extend_from_slice(&row);
        }
        let matrix = Array2::from_shape_vec((n, dim), data).unwrap();
        let ids: Vec<i64> = (1..=n as i64).collect();
        (matrix, ids)
    }

    #[test]
    fn binary_compression_32x() {
        let (matrix, ids) = make_normalized_matrix(100, 384);
        let bi = BinaryIndex::from_matrix(&matrix, &ids, false);
        let ratio = bi.compression_ratio();
        assert!(
            ratio > 30.0,
            "binary compression should be ~32×, got {ratio}"
        );
    }

    #[test]
    fn binary_search_finds_self() {
        let (matrix, ids) = make_normalized_matrix(100, 384);
        let bi = BinaryIndex::from_matrix(&matrix, &ids, false);

        let query: Vec<f32> = matrix.row(42).to_vec();
        let results = bi.search_binary(&query, 5);
        assert_eq!(results.len(), 5);
        // The query vector itself should be nearest (Hamming = 0)
        assert_eq!(results[0].0, 43, "should find doc 43 (id=42+1) first");
        assert_eq!(results[0].1, 0, "Hamming distance to self should be 0");
    }

    #[test]
    fn binary_twophase_recall() {
        let (matrix, ids) = make_normalized_matrix(200, 384);
        let bi = BinaryIndex::from_matrix(&matrix, &ids, true);
        let k = 10;

        // Ground truth via full f32 search
        let query: Vec<f32> = matrix.row(50).to_vec();
        let q_norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        let q_n: Vec<f32> = query.iter().map(|x| x / q_norm).collect();
        let mut gt: Vec<(i64, f32)> = (0..200)
            .map(|i| {
                let row = matrix.row(i);
                let dot: f32 = row.iter().zip(q_n.iter()).map(|(a, b)| a * b).sum();
                (ids[i], 1.0 - dot)
            })
            .collect();
        gt.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let gt_ids: Vec<i64> = gt.iter().take(k).map(|(id, _)| *id).collect();

        let results: Vec<i64> = bi
            .search_twophase(&query, k, 10)
            .iter()
            .map(|(id, _)| *id)
            .collect();
        let overlap = results.iter().filter(|id| gt_ids.contains(id)).count();
        let recall = overlap as f32 / k as f32;
        assert!(
            recall >= 0.7,
            "two-phase recall@{k} should be >= 0.7, got {recall} (gt={gt_ids:?}, got={results:?})"
        );
    }

    #[test]
    fn binarize_correctness() {
        let v = vec![1.0, -1.0, 0.5, -0.5, 0.0, 0.1, -0.1, 0.01];
        let bits = binarize(&v);
        assert_eq!(bits.len(), 1);
        // Positive at indices 0,2,5,7 → bits set: 0b10100101 = 165
        assert_eq!(bits[0], 0b10100101);
    }

    #[test]
    fn hamming_basic() {
        let a = vec![0b11110000u8];
        let b = vec![0b11001100u8];
        assert_eq!(hamming_distance(&a, &b), 4); // XOR = 0b00111100, popcount = 4
    }

    #[test]
    fn binary_zero_vector() {
        let (matrix, ids) = make_normalized_matrix(50, 384);
        let bi = BinaryIndex::from_matrix(&matrix, &ids, true);
        assert!(bi.search_twophase(&vec![0.0f32; 384], 5, 10).is_empty());
    }
}
