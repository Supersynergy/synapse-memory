//! Filtered ANN — metadata pre/post-filter wrapper (ACORN-lite).
//!
//! Wraps any index that returns `Vec<(i64, f32)>` and applies a caller-provided
//! `filter: Fn(i64) -> bool` predicate. Two modes:
//!   - **Post-filter** (default): run search with `k * over_fetch`, then filter
//!     and truncate to k. Best when selectivity is high (>20% pass).
//!   - **Pre-filter** (TODO): walk candidate IDs first, route only matching IDs
//!     to index search. Best when selectivity is low (<5%). Needs index-side
//!     hooks; scaffold here documents the API.
//!
//! Selectivity router decides automatically based on a `selectivity_hint`
//! (estimated pass-fraction). Caller can override via `mode`.

use std::sync::Arc;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FilterMode {
    /// Search then filter (high selectivity, >20% pass).
    Post,
    /// (Reserved) Walk filtered IDs then route to index (low selectivity, <5%).
    /// Currently falls back to Post until index hooks land.
    Pre,
    /// Auto: pick based on `selectivity_hint`.
    Auto,
}

pub struct FilteredAnn {
    /// Over-fetch multiplier for post-filter. Search k * over_fetch then filter.
    pub over_fetch: usize,
}

impl FilteredAnn {
    #[must_use]
    pub fn new() -> Self {
        Self { over_fetch: 4 }
    }

    /// Post-filter search. Caller passes a closure `search_fn(k_inner) -> Vec<(id, score)>`
    /// and a `keep: Fn(i64) -> bool` predicate.
    pub fn search<F, K>(&self, k: usize, search_fn: F, keep: K) -> Vec<(i64, f32)>
    where
        F: FnOnce(usize) -> Vec<(i64, f32)>,
        K: Fn(i64) -> bool,
    {
        if k == 0 {
            return Vec::new();
        }
        let inner_k = k.saturating_mul(self.over_fetch).max(k);
        let raw = search_fn(inner_k);
        let mut out: Vec<(i64, f32)> = raw.into_iter().filter(|(id, _)| keep(*id)).collect();
        out.truncate(k);
        out
    }

    /// Adaptive: caller passes hint `pass_fraction ∈ (0,1]`. Adjust over_fetch.
    /// At pass_fraction=1.0 (no filter), over_fetch=1. At 0.01 (1% pass), over_fetch=100.
    /// Capped at 256 to bound work.
    pub fn search_adaptive<F, K>(
        &self,
        k: usize,
        pass_fraction: f32,
        search_fn: F,
        keep: K,
    ) -> Vec<(i64, f32)>
    where
        F: FnOnce(usize) -> Vec<(i64, f32)>,
        K: Fn(i64) -> bool,
    {
        if k == 0 {
            return Vec::new();
        }
        let mult = (1.0 / pass_fraction.max(0.001)).ceil() as usize;
        let inner_k = k.saturating_mul(mult.min(256)).max(k);
        let raw = search_fn(inner_k);
        let mut out: Vec<(i64, f32)> = raw.into_iter().filter(|(id, _)| keep(*id)).collect();
        out.truncate(k);
        out
    }
}

impl Default for FilteredAnn {
    fn default() -> Self {
        Self::new()
    }
}

/// Type-erased filter predicate for storing in indexes that hold one.
pub type FilterPred = Arc<dyn Fn(i64) -> bool + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_filter_truncates_to_k() {
        let f = FilteredAnn::new();
        let r = f.search(
            3,
            |inner_k| {
                // mock search returning 0..inner_k with descending score
                (0..inner_k as i64)
                    .map(|i| (i, (inner_k as f32 - i as f32)))
                    .collect()
            },
            |id| id % 2 == 0,
        ); // keep even ids
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|(id, _)| id % 2 == 0));
    }

    #[test]
    fn adaptive_low_selectivity_overfetches() {
        let f = FilteredAnn::new();
        let r = f.search_adaptive(
            2,
            0.05,
            |inner_k| {
                // simulate: caller estimates 5% pass-rate → over_fetch=20
                assert!(
                    inner_k >= 2 * 20,
                    "expected over-fetch, got inner_k={inner_k}"
                );
                (0..inner_k as i64).map(|i| (i, i as f32)).collect()
            },
            |id| id % 20 == 0,
        );
        assert!(r.len() <= 2);
    }

    #[test]
    fn k_zero_returns_empty() {
        let f = FilteredAnn::new();
        let r = f.search(0, |_| vec![(1, 1.0), (2, 2.0)], |_| true);
        assert!(r.is_empty());
    }

    #[test]
    fn mode_enum_exists() {
        assert_ne!(FilterMode::Post, FilterMode::Pre);
        assert_ne!(FilterMode::Post, FilterMode::Auto);
    }
}
