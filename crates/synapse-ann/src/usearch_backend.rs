//! usearch backend — PR-A1 of scale-100M plan.
//!
//! Thin adapter over the `usearch` crate implementing `AnnIndex`.
//! Default: MetricKind::Cos, ScalarKind::F32, HNSW.
//! Scalar quantization (i8/bf16) is available post-PR-C1 when synapse-quant
//! lands; for now we stay f32 to match the ladder fairness assumption.

use crate::{AnnError, AnnIndex};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

pub struct UsearchIndex {
    idx: Index,
    dim: usize,
    len: usize,
}

impl UsearchIndex {
    /// Build a new HNSW index. `expected_capacity` sizes the internal arrays
    /// once up-front — over-sizing costs RAM, under-sizing forces realloc.
    pub fn new(dim: usize, expected_capacity: usize) -> Result<Self, AnnError> {
        let opts = IndexOptions {
            dimensions: dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 16, // HNSW M; usearch default
            expansion_add: 128,
            expansion_search: 64,
            multi: false,
        };
        let idx =
            Index::new(&opts).map_err(|e| AnnError::Other(format!("usearch new: {e:?}")))?;
        idx.reserve(expected_capacity.max(1024))
            .map_err(|e| AnnError::Other(format!("usearch reserve: {e:?}")))?;
        Ok(Self { idx, dim, len: 0 })
    }
}

impl AnnIndex for UsearchIndex {
    fn insert(&mut self, id: u64, vector: &[f32]) -> Result<(), AnnError> {
        if vector.len() != self.dim {
            return Err(AnnError::DimMismatch {
                expected: self.dim,
                actual: vector.len(),
            });
        }
        self.idx
            .add(id, vector)
            .map_err(|e| AnnError::Other(format!("usearch add: {e:?}")))?;
        self.len += 1;
        Ok(())
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>, AnnError> {
        if query.len() != self.dim {
            return Err(AnnError::DimMismatch {
                expected: self.dim,
                actual: query.len(),
            });
        }
        let matches = self
            .idx
            .search(query, k)
            .map_err(|e| AnnError::Other(format!("usearch search: {e:?}")))?;
        Ok(matches
            .keys
            .into_iter()
            .zip(matches.distances)
            .collect())
    }

    fn len(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(seed: u64, dim: usize) -> Vec<f32> {
        (0..dim)
            .map(|i| ((seed.wrapping_mul(13) + i as u64).wrapping_mul(7) % 997) as f32 / 997.0 - 0.5)
            .collect()
    }

    #[test]
    fn insert_and_search_round_trip() {
        let mut idx = UsearchIndex::new(64, 1024).unwrap();
        for i in 0..500u64 {
            idx.insert(i, &v(i, 64)).unwrap();
        }
        assert_eq!(idx.len(), 500);
        let q = v(42, 64);
        let hits = idx.search(&q, 5).unwrap();
        assert_eq!(hits.len(), 5);
        // id 42 must be in top-1 (distance ≈ 0 vs itself).
        assert_eq!(hits[0].0, 42);
    }

    #[test]
    fn dim_mismatch_rejected() {
        let mut idx = UsearchIndex::new(64, 16).unwrap();
        let err = idx.insert(0, &[0.0; 32]).unwrap_err();
        matches!(err, AnnError::DimMismatch { .. });
    }
}
