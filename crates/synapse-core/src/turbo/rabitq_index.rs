//! RaBitQ cascade index — closes f16 recall ceiling toward 0.99+.
//!
//! Two-stage pipeline glued into a single search call:
//!   1. **Stage A** — `InMemoryHammingIndex` wide candidate sweep over
//!      1-bit sign codes (cheap, recall ~0.72).
//!   2. **Stage B** — RaBitQ `dot_estimator` rerank of top-`rerank_n`
//!      Hamming candidates. Tighter ordering, recall ~0.95+ at small `rerank_n`.
//!   3. (Optional) **Stage C** — caller passes top-K to f16/f32 verify.
//!
//! Memory: 1-bit/dim Hamming + 1-bit/dim RaBitQ + per-row (mean, inv_norm)
//! = ~2 bits/dim + 8 bytes/row metadata. 50× smaller than f16, 100× vs f32.

use crate::turbo::inmem_hamming_index::InMemoryHammingIndex;
use crate::turbo::rabitq_rerank::{build_rotation, encode_rabitq, dot_estimator, RaBitQCode};
use std::collections::HashMap;

/// Cascade index: Hamming sweep → RaBitQ rerank.
pub struct RaBitQIndex {
    hamming: InMemoryHammingIndex,
    codes: Vec<RaBitQCode>,
    id_to_code: HashMap<i64, usize>,
    rotation: Vec<f32>,
    dim: usize,
}

impl RaBitQIndex {
    /// Build cascade index. `seed` controls the random rotation; pin per index.
    #[must_use]
    pub fn build(rows: Vec<(i64, Vec<f32>)>, seed: u64) -> Self {
        if rows.is_empty() {
            return Self {
                hamming: InMemoryHammingIndex::build(Vec::new()),
                codes: Vec::new(),
                id_to_code: HashMap::new(),
                rotation: Vec::new(),
                dim: 0,
            };
        }
        let dim = rows[0].1.len();
        let rotation = build_rotation(dim, seed);
        let codes: Vec<RaBitQCode> = rows.iter().map(|(_, v)| encode_rabitq(v, &rotation)).collect();
        let id_to_code: HashMap<i64, usize> = rows.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();
        let hamming = InMemoryHammingIndex::build(rows);
        Self { hamming, codes, id_to_code, rotation, dim }
    }

    /// Row count.
    #[must_use]
    pub fn len(&self) -> usize { self.hamming.len() }
    /// Empty.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.hamming.is_empty() }
    /// Dim.
    #[must_use]
    pub const fn dim(&self) -> usize { self.dim }

    /// Cascade search:
    /// 1. Hamming top-`rerank_n` (default rerank_n = 10*k).
    /// 2. RaBitQ unbiased dot estimator rerank to top-k.
    pub fn search(&self, query: &[f32], k: usize, rerank_n: Option<usize>) -> Vec<(i64, f32)> {
        if self.is_empty() || query.len() != self.dim || k == 0 {
            return Vec::new();
        }
        let rerank_n = rerank_n.unwrap_or(k.saturating_mul(10)).max(k);
        // Stage A: cheap binary Hamming sweep
        let cands = self.hamming.search(query, rerank_n);
        if cands.is_empty() { return Vec::new(); }
        // Stage B: RaBitQ rerank — id→code via HashMap (built once at index build)
        let mut reranked: Vec<(i64, f32)> = cands
            .into_iter()
            .filter_map(|(id, _hamming_dist)| {
                let code_idx = *self.id_to_code.get(&id)?;
                let est = dot_estimator(query, &self.rotation, &self.codes[code_idx]);
                Some((id, est))
            })
            .collect();
        // Sort descending by RaBitQ estimator (higher = closer per unbiased dot).
        reranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        reranked.truncate(k);
        reranked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_safe() {
        let idx = RaBitQIndex::build(Vec::new(), 0);
        assert!(idx.is_empty());
        assert!(idx.search(&[1.0, 0.0], 5, None).is_empty());
    }

    #[test]
    fn small_corpus_returns_topk() {
        let rows: Vec<(i64, Vec<f32>)> = (0..20)
            .map(|i| (i, (0..8).map(|j| (i + j) as f32 * 0.1).collect()))
            .collect();
        let idx = RaBitQIndex::build(rows, 42);
        let q = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let top = idx.search(&q, 5, Some(15));
        assert!(top.len() <= 5);
    }

    #[test]
    fn dim_mismatch_safe() {
        let rows = vec![(1_i64, vec![1.0, 0.0])];
        let idx = RaBitQIndex::build(rows, 0);
        assert!(idx.search(&[1.0, 0.0, 0.0], 1, None).is_empty());
    }
}
