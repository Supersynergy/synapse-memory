//! One-call multi-index builder — I8 + F16 + Hamming behind a router.
//!
//! Instead of the user managing three indices + picking paths, `MultiIndex`
//! wraps them plus an [`AdaptiveRouter`]. Each `search` call consults the
//! router with the corpus size + user hints and dispatches to the best
//! backend. Observed latency + recall can be fed back via `observe`.
//!
//! ```
//! # #[cfg(feature = "simsimd")] {
//! use synapse_core::turbo::multi_index::{MultiIndex, SearchHints};
//! let rows = vec![(1_i64, vec![1.0_f32, 0.0, 0.0, 0.0])];
//! let idx = MultiIndex::build(rows);
//! let hits = idx.search(&[1.0, 0.0, 0.0, 0.0], SearchHints::default());
//! assert!(!hits.is_empty());
//! # }
//! ```

#![allow(clippy::type_complexity)]

use std::sync::Mutex;

use crate::turbo::adaptive_router::{AdaptiveRouter, QueryHints, Strategy};
use crate::turbo::inmem_f16_index::InMemoryF16Index;
use crate::turbo::inmem_hamming_index::InMemoryHammingIndex;
use crate::turbo::inmem_i8_index::InMemoryI8Index;
use crate::turbo::rabitq_index::RaBitQIndex;

/// Per-query hints forwarded to [`AdaptiveRouter`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchHints {
    /// Soft latency budget in µs (0 = no hint).
    pub latency_budget_us: u64,
    /// Required recall@10, 0..1 (0 = no constraint).
    pub min_recall: f64,
    /// Requested top-k (default 10).
    pub k: usize,
}

/// Bundle of all in-memory backends + an adaptive router.
pub struct MultiIndex {
    i8_idx: InMemoryI8Index,
    f16_idx: InMemoryF16Index,
    ham_idx: InMemoryHammingIndex,
    rabitq_idx: RaBitQIndex,
    router: Mutex<AdaptiveRouter>,
    n: usize,
}

impl MultiIndex {
    /// Build all three backends from the same `(id, Vec<f32>)` corpus.
    ///
    /// # Note on memory
    /// The current impl clones the input twice so the three backend
    /// constructors stay independent — build-time peak is `3 × N × D × 4`
    /// bytes, dropping to `N × D × (1 + 2 + bpr/8)` after. For a 100 k × 384
    /// corpus that's a short ~460 MB peak, falling to ~100 MB steady. A
    /// fused-single-pass builder is tracked as a future optimization.
    #[must_use]
    pub fn build(rows: Vec<(i64, Vec<f32>)>) -> Self {
        let n = rows.len();
        Self {
            i8_idx: InMemoryI8Index::build(rows.clone()),
            f16_idx: InMemoryF16Index::build(rows.clone()),
            ham_idx: InMemoryHammingIndex::build(rows.clone()),
            rabitq_idx: RaBitQIndex::build(rows, 0x00BA_1B17_5EED_u64),
            router: Mutex::new(AdaptiveRouter::new()),
            n,
        }
    }

    /// Row count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }
    /// Empty probe.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Ask the router and dispatch. Falls back to [`Strategy::SimSimdI8`]
    /// when the chosen strategy isn't in-memory (e.g. RayonF32, ScalarF32
    /// aren't backed by this bundle yet — int8 is the safe all-round default).
    #[must_use = "search returns ranked hits; discarding silently is a bug"]
    pub fn search(&self, query: &[f32], hints: SearchHints) -> Vec<(i64, f32)> {
        let k = if hints.k == 0 { 10 } else { hints.k };
        let chosen = match self.router.lock() {
            Ok(g) => g.choose(&QueryHints {
                corpus_size: self.n,
                latency_budget_us: hints.latency_budget_us,
                min_recall: hints.min_recall,
            }),
            // Poisoned router → fall back to the default-safe strategy rather
            // than propagating — search must remain available on lock recovery.
            Err(_) => Strategy::SimSimdI8,
        };
        match chosen {
            Strategy::SimSimdHamming => {
                let h = self.ham_idx.search(query, k);
                h.into_iter().map(|(id, d)| (id, -(d as f32))).collect()
            }
            Strategy::SimSimdF32
            | Strategy::MrlSimSimd
            | Strategy::RayonF32
            | Strategy::ScalarF32 => {
                // f16 is the closest full-recall path backed here.
                self.f16_idx.search(query, k)
            }
            Strategy::SimSimdI8 => self.i8_idx.search(query, k),
            Strategy::RaBitQCascade => self.rabitq_idx.search(query, k, None),
        }
    }

    /// Feed back observed latency + recall so the router improves. Silent on
    /// poisoned lock (diagnostics belong in the caller, not here).
    pub fn observe(&self, strat: Strategy, us: f64, recall: f64) {
        if let Ok(mut g) = self.router.lock() {
            g.observe(strat, us, recall);
        }
    }

    /// Current router posterior — for UIs/dashboards.
    pub fn router_posterior(&self) -> Vec<(Strategy, f64, f64)> {
        self.router
            .lock()
            .map(|g| g.posterior_means())
            .unwrap_or_default()
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
    fn build_and_search_returns_top1_exact() {
        let rows = vec![
            (10_i64, unit(vec![1.0, 0.0, 0.0, 0.0])),
            (20, unit(vec![0.0, 1.0, 0.0, 0.0])),
            (30, unit(vec![0.0, 0.0, 1.0, 0.0])),
        ];
        let idx = MultiIndex::build(rows);
        let hits = idx.search(&unit(vec![1.0, 0.0, 0.0, 0.0]), SearchHints::default());
        assert_eq!(hits.first().map(|(id, _)| *id), Some(10));
    }

    #[test]
    fn empty_index_returns_empty() {
        let idx = MultiIndex::build(Vec::new());
        assert!(idx.is_empty());
        assert!(idx.search(&[1.0, 0.0], SearchHints::default()).is_empty());
    }

    #[test]
    fn observe_feeds_through_to_posterior() {
        let idx = MultiIndex::build(vec![(1_i64, unit(vec![1.0, 0.0, 0.0, 0.0]))]);
        for _ in 0..10 {
            idx.observe(Strategy::SimSimdI8, 300.0, 0.98);
        }
        let post = idx.router_posterior();
        let int8_entry = post
            .iter()
            .find(|(s, _, _)| matches!(s, Strategy::SimSimdI8));
        assert!(int8_entry.is_some());
    }
}
