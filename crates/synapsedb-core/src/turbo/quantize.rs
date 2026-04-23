//! Scalar Quantization — 4× memory reduction for vector search
//!
//! Compresses f32 vectors to i8 with per-dimension min/max codebook.
//! Roundtrip error per dimension: < 0.02 (acceptable for cosine ranking).
//!
//! Memory savings (384-dim):
//!   - f32: 1536 bytes/vector
//!   - i8:   384 bytes/vector (4× smaller)
//!
//! Usage:
//! ```ignore
//! let quantized = QuantizedSearch::from_ndarray(&ndarray_search);
//! let results = quantized.search(&query_emb, 10);
//! ```

use super::simd;

/// Per-dimension codebook for scalar int8 quantization.
#[derive(Debug, Clone)]
pub struct ScalarQuantizer {
    mins: Vec<f32>,
    maxs: Vec<f32>,
    dim: usize,
    /// Pre-computed: average self-dot of quantized normalized vectors.
    /// Used to convert i8 dot products to approximate cosine similarity.
    norm_factor: f64,
}

impl ScalarQuantizer {
    /// Train codebook from a matrix of vectors (typically pre-normalized).
    pub fn train(vectors: &ndarray::Array2<f32>) -> Self {
        let dim = vectors.ncols();
        let n = vectors.nrows();
        let mut mins = vec![f32::INFINITY; dim];
        let mut maxs = vec![f32::NEG_INFINITY; dim];

        for row in vectors.rows() {
            for (j, &val) in row.iter().enumerate() {
                if val < mins[j] {
                    mins[j] = val;
                }
                if val > maxs[j] {
                    maxs[j] = val;
                }
            }
        }

        // Avoid zero range (degenerate dimensions)
        for j in 0..dim {
            if (maxs[j] - mins[j]).abs() < 1e-10 {
                mins[j] -= 0.5;
                maxs[j] += 0.5;
            }
        }

        // Compute normalization factor: average self-dot of quantized vectors.
        // For normalized f32 vectors, self-dot = 1.0, so this gives us the
        // scale at which i8_dot ≈ 1.0.
        let mut sum_self_dot: i64 = 0;
        for row in vectors.rows() {
            let encoded = Self::encode_raw(row.as_slice().unwrap(), &mins, &maxs);
            let sd: i64 = encoded
                .iter()
                .map(|&x| (x as i64) * (x as i64))
                .sum();
            sum_self_dot += sd;
        }
        let norm_factor = if n > 0 {
            sum_self_dot as f64 / n as f64
        } else {
            1.0
        };

        Self {
            mins,
            maxs,
            dim,
            norm_factor,
        }
    }

    /// Encode a single f32 vector to i8.
    pub fn encode(&self, v: &[f32]) -> Vec<i8> {
        Self::encode_raw(v, &self.mins, &self.maxs)
    }

    /// Decode a single i8 vector back to approximate f32.
    pub fn decode(&self, q: &[i8]) -> Vec<f32> {
        q.iter()
            .enumerate()
            .map(|(j, &val)| {
                let norm = (val as f32 + 128.0) / 255.0;
                self.mins[j] + norm * (self.maxs[j] - self.mins[j])
            })
            .collect()
    }

    /// Dimension of the codebook.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Normalization factor for converting i8 dot → approximate cosine.
    pub fn norm_factor(&self) -> f64 {
        self.norm_factor
    }

    /// Roundtrip error: encode then decode, return max absolute difference.
    pub fn roundtrip_error(&self, v: &[f32]) -> f32 {
        let encoded = self.encode(v);
        let decoded = self.decode(&encoded);
        v.iter()
            .zip(decoded.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max)
    }

    // Internal: encode without needing &self (used during training)
    fn encode_raw(v: &[f32], mins: &[f32], maxs: &[f32]) -> Vec<i8> {
        v.iter()
            .enumerate()
            .map(|(j, &val)| {
                let range = maxs[j] - mins[j];
                let norm = (val - mins[j]) / range;
                let clamped = norm.clamp(0.0, 1.0);
                (clamped * 255.0 - 128.0) as i8
            })
            .collect()
    }
}

/// Quantized vector index for fast int8 search.
///
/// Stores vectors as i8 with a codebook for encoding queries.
/// Search uses SIMD i8 dot product (NEON on ARM64).
pub struct QuantizedSearch {
    /// Flat i8 matrix: [n_vectors * dim]
    data: Vec<i8>,
    /// Document IDs corresponding to each row
    ids: Vec<i64>,
    n_vectors: usize,
    dim: usize,
    /// Codebook for encoding queries and converting scores
    pub codebook: ScalarQuantizer,
}

impl QuantizedSearch {
    /// Build quantized index from an f32 matrix and document IDs.
    pub fn from_matrix(matrix: &ndarray::Array2<f32>, ids: &[i64]) -> Self {
        let codebook = ScalarQuantizer::train(matrix);
        let n_vectors = matrix.nrows();
        let dim = matrix.ncols();

        // Encode all vectors to flat i8 buffer
        let mut data = Vec::with_capacity(n_vectors * dim);
        for row in matrix.rows() {
            data.extend_from_slice(&codebook.encode(row.as_slice().unwrap()));
        }

        Self {
            data,
            ids: ids.to_vec(),
            n_vectors,
            dim,
            codebook,
        }
    }

    /// Search for k nearest neighbors using i8 dot product.
    /// Returns (doc_id, approximate_distance) sorted ascending by distance.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        if query.len() != self.dim || self.n_vectors == 0 {
            return Vec::new();
        }

        // Normalize query
        let q_norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        if q_norm < 1e-10 {
            return Vec::new();
        }
        let q_normalized: Vec<f32> = query.iter().map(|x| x / q_norm).collect();

        // Quantize query
        let q_i8 = self.codebook.encode(&q_normalized);

        let k = k.min(self.n_vectors);
        if k == 0 {
            return Vec::new();
        }

        // Compute all i8 dot products via SIMD batch
        let dots = simd::dot_batch_i8(&self.data, &q_i8, self.dim, self.n_vectors);

        // Find top-k by highest dot product (= most similar)
        let mut indices: Vec<usize> = (0..self.n_vectors).collect();
        indices.select_nth_unstable_by(k - 1, |&a, &b| dots[b].cmp(&dots[a]));

        let norm = self.codebook.norm_factor();

        let mut results: Vec<(i64, f32)> = indices[..k]
            .iter()
            .map(|&idx| {
                let cosine_approx = dots[idx] as f64 / norm;
                let distance = (1.0 - cosine_approx).max(0.0) as f32;
                (self.ids[idx], distance)
            })
            .collect();

        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Number of vectors in the index.
    pub fn len(&self) -> usize {
        self.n_vectors
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.n_vectors == 0
    }

    /// Memory usage in bytes (data + ids + codebook overhead).
    pub fn memory_bytes(&self) -> usize {
        self.data.len() + self.ids.len() * 8 + self.dim * 8 + 8
    }

    /// Compression ratio vs f32 storage.
    pub fn compression_ratio(&self) -> f32 {
        let f32_bytes = self.n_vectors * self.dim * 4;
        if f32_bytes == 0 {
            return 1.0;
        }
        f32_bytes as f32 / self.data.len() as f32
    }
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
    fn quantize_roundtrip_error() {
        let (matrix, _) = make_normalized_matrix(100, 384);
        let codebook = ScalarQuantizer::train(&matrix);

        for row in matrix.rows() {
            let err = codebook.roundtrip_error(row.as_slice().unwrap());
            assert!(
                err < 0.05,
                "roundtrip error {err} exceeds threshold 0.05"
            );
        }
    }

    #[test]
    fn quantize_encode_decode_dims() {
        let (matrix, _) = make_normalized_matrix(10, 384);
        let codebook = ScalarQuantizer::train(&matrix);
        let row = matrix.row(0);
        let encoded = codebook.encode(row.as_slice().unwrap());
        assert_eq!(encoded.len(), 384);
        let decoded = codebook.decode(&encoded);
        assert_eq!(decoded.len(), 384);
    }

    #[test]
    fn quantized_search_recall() {
        let (matrix, ids) = make_normalized_matrix(200, 384);
        let qs = QuantizedSearch::from_matrix(&matrix, &ids);

        // Query with the first vector — should find itself as nearest
        let query: Vec<f32> = matrix.row(0).to_vec();
        let results = qs.search(&query, 5);
        assert_eq!(results.len(), 5);
        assert_eq!(results[0].0, 1, "nearest should be doc 1 (self)");
        assert!(results[0].1 < 0.1, "distance to self should be small");
    }

    #[test]
    fn quantized_search_recall_at_k() {
        let (matrix, ids) = make_normalized_matrix(200, 384);
        let qs = QuantizedSearch::from_matrix(&matrix, &ids);

        // Brute-force ground truth
        let query: Vec<f32> = matrix.row(42).to_vec();
        let k = 10;

        // Ground truth via f32 dot products
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

        let results: Vec<i64> = qs.search(&query, k).iter().map(|(id, _)| *id).collect();
        let overlap = results.iter().filter(|id| gt_ids.contains(id)).count();
        let recall = overlap as f32 / k as f32;
        assert!(
            recall >= 0.8,
            "recall@{k} should be >= 0.8, got {recall} (gt={gt_ids:?}, qs={results:?})"
        );
    }

    #[test]
    fn quantized_search_zero_vector() {
        let (matrix, ids) = make_normalized_matrix(50, 384);
        let qs = QuantizedSearch::from_matrix(&matrix, &ids);
        let zero = vec![0.0f32; 384];
        assert!(qs.search(&zero, 5).is_empty());
    }

    #[test]
    fn quantized_search_dim_mismatch() {
        let (matrix, ids) = make_normalized_matrix(50, 384);
        let qs = QuantizedSearch::from_matrix(&matrix, &ids);
        let wrong = vec![1.0f32; 128];
        assert!(qs.search(&wrong, 5).is_empty());
    }

    #[test]
    fn quantized_compression_ratio() {
        let (matrix, ids) = make_normalized_matrix(100, 384);
        let qs = QuantizedSearch::from_matrix(&matrix, &ids);
        let ratio = qs.compression_ratio();
        assert!(
            (ratio - 4.0).abs() < 0.01,
            "compression ratio should be ~4.0, got {ratio}"
        );
    }

    #[test]
    fn quantized_memory_savings() {
        let (matrix, ids) = make_normalized_matrix(1000, 384);
        let qs = QuantizedSearch::from_matrix(&matrix, &ids);
        let f32_bytes = 1000 * 384 * 4; // 1,536,000
        let q_bytes = qs.memory_bytes();
        assert!(
            q_bytes < f32_bytes / 2,
            "quantized should use < half the memory: {q_bytes} vs {f32_bytes}"
        );
    }
}
