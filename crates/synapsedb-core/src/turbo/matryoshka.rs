//! Matryoshka Funnel Search — adaptive-dimension two-phase search.
//!
//! Inspired by Matryoshka Representation Learning (MRL): embeddings trained
//! so that the first `d` dimensions form a valid lower-dimensional embedding.
//!
//! Two-phase funnel search:
//! 1. **Coarse pass**: Truncate all vectors to `coarse_dim` (e.g. 48), brute-force
//!    top `funnel_k` candidates. This is cheap because we touch 8× less data.
//! 2. **Refine pass**: Re-score only the candidates at full dimensionality,
//!    return top `k`.
//!
//! Pattern validated from:
//! - ruvnet/RuVector `matryoshka.rs`: `MatryoshkaIndex::funnel_search`
//! - Dicklesworthstone/xf `static_mrl_embedder.rs`: `truncate_embedding()` + re-normalize
//! - vllm-project/semantic-router: mmBERT 2D Matryoshka (768 → 64..768)

use super::simd;

/// Configuration for Matryoshka funnel search.
#[derive(Debug, Clone)]
pub struct MatryoshkaConfig {
    /// Dimension for the coarse (fast) pass. Must be <= full dim.
    pub coarse_dim: usize,
    /// How many candidates to retrieve in coarse pass (multiplier on k).
    pub funnel_factor: usize,
}

impl Default for MatryoshkaConfig {
    /// Defaults tuned on real BGE-small-en-v1.5 embeddings (5000 docs, 384-dim):
    /// coarse_dim=48, funnel_factor=2 → 16.89µs @ 1.0000 recall@10.
    fn default() -> Self {
        Self {
            coarse_dim: 48,
            funnel_factor: 2,
        }
    }
}

/// Matryoshka-aware search index wrapping a flat f32 matrix.
///
/// Stores the full-dimensional matrix but can search at any prefix dimension.
pub struct MatryoshkaSearch {
    /// Row-major flat f32 matrix [n_vectors * full_dim]
    data: Vec<f32>,
    /// Document IDs
    ids: Vec<i64>,
    n_vectors: usize,
    full_dim: usize,
    config: MatryoshkaConfig,
}

impl MatryoshkaSearch {
    /// Build from an ndarray matrix (already pre-normalized).
    pub fn from_matrix(
        matrix: &ndarray::Array2<f32>,
        ids: &[i64],
        config: MatryoshkaConfig,
    ) -> Self {
        let n_vectors = matrix.nrows();
        let full_dim = matrix.ncols();
        let data = matrix
            .as_slice()
            .map(|s| s.to_vec())
            .unwrap_or_else(|| {
                let mut v = Vec::with_capacity(n_vectors * full_dim);
                for row in matrix.rows() {
                    v.extend_from_slice(row.as_slice().unwrap());
                }
                v
            });

        Self {
            data,
            ids: ids.to_vec(),
            n_vectors,
            full_dim,
            config,
        }
    }

    /// Two-phase funnel search.
    ///
    /// 1. Coarse: truncated prefix dim → top `k * funnel_factor` candidates
    /// 2. Refine: full dim → top `k`
    pub fn funnel_search(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        if query.len() != self.full_dim || self.n_vectors == 0 {
            return Vec::new();
        }

        // Normalize full query
        let q_norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        if q_norm < 1e-10 {
            return Vec::new();
        }
        let q_full: Vec<f32> = query.iter().map(|x| x / q_norm).collect();

        let k = k.min(self.n_vectors);
        if k == 0 {
            return Vec::new();
        }

        let coarse_dim = self.config.coarse_dim.min(self.full_dim);
        let funnel_k = (k * self.config.funnel_factor).min(self.n_vectors);

        // ── Phase 1: Coarse pass at truncated dimension ──
        // Truncate query and re-normalize
        let q_coarse = truncate_and_normalize(&q_full[..coarse_dim]);

        // Compute coarse similarities using only the first `coarse_dim` elements
        let mut coarse_scores: Vec<(usize, f32)> = (0..self.n_vectors)
            .map(|i| {
                let offset = i * self.full_dim;
                let row_prefix = &self.data[offset..offset + coarse_dim];
                // Re-normalize the prefix for valid cosine
                let row_norm: f32 = row_prefix.iter().map(|x| x * x).sum::<f32>().sqrt();
                let sim = if row_norm > 1e-10 {
                    simd::dot_f32(row_prefix, &q_coarse) / row_norm
                } else {
                    0.0
                };
                (i, sim)
            })
            .collect();

        // Partial sort to get top funnel_k candidates
        coarse_scores.select_nth_unstable_by(funnel_k - 1, |a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        let candidates = &coarse_scores[..funnel_k];

        // ── Phase 2: Refine at full dimension ──
        let mut refined: Vec<(i64, f32)> = candidates
            .iter()
            .map(|&(idx, _)| {
                let offset = idx * self.full_dim;
                let row = &self.data[offset..offset + self.full_dim];
                let sim = simd::dot_f32(row, &q_full);
                (self.ids[idx], 1.0 - sim) // distance
            })
            .collect();

        // Partial sort top-k from refined candidates
        if refined.len() > k {
            refined.select_nth_unstable_by(k - 1, |a, b| {
                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
            });
            refined.truncate(k);
        }
        refined.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        refined
    }

    /// Direct full-dimension search (for comparison/baseline).
    pub fn full_search(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        if query.len() != self.full_dim || self.n_vectors == 0 {
            return Vec::new();
        }

        let q_norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        if q_norm < 1e-10 {
            return Vec::new();
        }
        let q_normalized: Vec<f32> = query.iter().map(|x| x / q_norm).collect();

        let similarities =
            simd::dot_batch_f32(&self.data, &q_normalized, self.full_dim, self.n_vectors);

        let k = k.min(self.n_vectors);
        if k == 0 {
            return Vec::new();
        }

        let mut indices: Vec<usize> = (0..self.n_vectors).collect();
        indices.select_nth_unstable_by(k - 1, |&a, &b| {
            similarities[b]
                .partial_cmp(&similarities[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut results: Vec<(i64, f32)> = indices[..k]
            .iter()
            .map(|&i| (self.ids[i], 1.0 - similarities[i]))
            .collect();
        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    pub fn len(&self) -> usize {
        self.n_vectors
    }

    pub fn is_empty(&self) -> bool {
        self.n_vectors == 0
    }

    pub fn config(&self) -> &MatryoshkaConfig {
        &self.config
    }
}

/// Truncate and re-normalize a vector prefix for valid cosine similarity.
/// Critical: after truncation, the vector is no longer unit-length.
fn truncate_and_normalize(prefix: &[f32]) -> Vec<f32> {
    let norm: f32 = prefix.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-10 {
        return prefix.to_vec();
    }
    prefix.iter().map(|x| x / norm).collect()
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
    fn matryoshka_funnel_basic() {
        let (matrix, ids) = make_normalized_matrix(200, 384);
        let ms = MatryoshkaSearch::from_matrix(&matrix, &ids, MatryoshkaConfig::default());

        let query: Vec<f32> = matrix.row(42).to_vec();
        let results = ms.funnel_search(&query, 5);
        assert_eq!(results.len(), 5);
        assert_eq!(results[0].0, 43, "should find doc 43 (id=row+1) first");
    }

    #[test]
    fn matryoshka_recall_vs_full() {
        let (matrix, ids) = make_normalized_matrix(500, 384);
        let ms = MatryoshkaSearch::from_matrix(&matrix, &ids, MatryoshkaConfig::default());
        let k = 10;

        let mut total_recall = 0.0;
        let seeds = [1, 50, 100, 200, 400];
        for &seed in &seeds {
            let query: Vec<f32> = matrix.row(seed).to_vec();
            let full_ids: Vec<i64> = ms.full_search(&query, k).iter().map(|(id, _)| *id).collect();
            let funnel_ids: Vec<i64> =
                ms.funnel_search(&query, k).iter().map(|(id, _)| *id).collect();

            let overlap = full_ids.iter().filter(|id| funnel_ids.contains(id)).count();
            let recall = overlap as f32 / k as f32;
            total_recall += recall;
        }

        let avg_recall = total_recall / seeds.len() as f32;
        assert!(
            avg_recall >= 0.8,
            "average funnel recall@{k} should be >= 0.8, got {avg_recall}"
        );
    }

    #[test]
    fn matryoshka_zero_vector() {
        let (matrix, ids) = make_normalized_matrix(50, 384);
        let ms = MatryoshkaSearch::from_matrix(&matrix, &ids, MatryoshkaConfig::default());
        assert!(ms.funnel_search(&vec![0.0f32; 384], 5).is_empty());
    }

    #[test]
    fn matryoshka_dim_mismatch() {
        let (matrix, ids) = make_normalized_matrix(50, 384);
        let ms = MatryoshkaSearch::from_matrix(&matrix, &ids, MatryoshkaConfig::default());
        assert!(ms.funnel_search(&vec![1.0f32; 128], 5).is_empty());
    }

    #[test]
    fn truncate_normalize_preserves_direction() {
        let v = vec![3.0, 4.0, 0.0, 0.0];
        let normed = truncate_and_normalize(&v[..2]);
        assert!((normed[0] - 0.6).abs() < 1e-5);
        assert!((normed[1] - 0.8).abs() < 1e-5);
    }
}
