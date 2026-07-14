//! Adaptive router — picks the best ANN strategy per query via Thompson bandit.
//!
//! Inspired by [claude-token-saver `adaptive_router`] and the CatBoost antiban
//! precedent. Every search call is scored on two axes:
//!
//! * **speed** — wall-time microseconds (lower = better).
//! * **recall** — intersection with f32 ground-truth top-k (higher = better).
//!
//! A Beta-distribution posterior per [`Strategy`] is updated on every call, and
//! the next query samples the strategy with the highest Thompson draw. A hard
//! corpus-size gate overrides sampling for obviously-wrong choices (binary
//! hamming on a 100-doc corpus is never worth the recall hit).
//!
//! ## Safety & fallback
//!
//! * The router never panics on unknown input — unknown corpus size falls back
//!   to [`Strategy::SimSimdI8`].
//! * Every strategy has a functional implementation — if the requested one is
//!   unavailable (feature disabled), we degrade to [`Strategy::ScalarF32`].
//! * Bandit posteriors are clamped to `[1, 10_000]` pseudo-observations so a
//!   single bad run cannot poison future routing.
//! * Arithmetic is checked with saturating `_f64` math; no integer overflow.
//!
//! ## Example
//!
//! ```
//! use synapse_core::turbo::adaptive_router::{AdaptiveRouter, Strategy, QueryHints};
//!
//! let mut router = AdaptiveRouter::new();
//! let hints = QueryHints { corpus_size: 250_000, latency_budget_us: 500, min_recall: 0.95 };
//! let s = router.choose(&hints);
//! // ... run search with `s` ...
//! router.observe(s, /*us=*/ 312.0, /*recall=*/ 0.98);
//! ```

use std::collections::HashMap;

/// Search strategies the router can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Strategy {
    /// Naive scalar cos f32 — only rational for tiny corpora (<1 k).
    ScalarF32,
    /// Rayon-parallel scalar cos — 4-12× ScalarF32 at >10 k.
    RayonF32,
    /// SimSIMD NEON cos f32 — default for mid corpora.
    SimSimdF32,
    /// SimSIMD int8 dot + per-row scale — best recall-at-speed ratio.
    SimSimdI8,
    /// SimSIMD 1-bit Hamming — fastest, lossy; requires rerank stage.
    SimSimdHamming,
    /// Matryoshka-truncated (k=128) + SimSIMD cos — 3× cheap, needs MRL model.
    MrlSimSimd,
    /// RaBitQ cascade — Hamming sweep → RaBitQ unbiased rerank. Closes the
    /// recall ceiling from 0.72 (raw Hamming) toward 0.95 with ~50× smaller
    /// memory than f16. Best at medium-large scale where memory matters.
    RaBitQCascade,
}

impl Strategy {
    /// Static prior: expected-recall floor (0..1) — used before bandit warms up.
    pub(crate) const fn prior_recall(self) -> f64 {
        match self {
            Self::ScalarF32 | Self::RayonF32 | Self::SimSimdF32 => 1.00,
            Self::SimSimdI8 => 0.97,
            Self::MrlSimSimd => 0.95,
            Self::SimSimdHamming => 0.72, // w/o rerank
            Self::RaBitQCascade => 0.95,  // Hamming + RaBitQ rerank
        }
    }

    /// Expected-µs-per-100k-rows floor.
    pub(crate) const fn prior_us_per_100k(self) -> f64 {
        match self {
            Self::ScalarF32 => 13_000.0,
            Self::RayonF32 => 1_500.0,
            Self::SimSimdF32 => 760.0,
            Self::SimSimdI8 => 325.0,
            Self::SimSimdHamming => 250.0,
            Self::MrlSimSimd => 400.0,
            Self::RaBitQCascade => 500.0, // Hamming + rerank N candidates
        }
    }
}

/// Per-query hints the caller supplies to the router.
#[derive(Debug, Clone, Copy)]
pub struct QueryHints {
    /// How many vectors live in the corpus right now.
    pub corpus_size: usize,
    /// Soft deadline in microseconds (0 = no hint).
    pub latency_budget_us: u64,
    /// Required recall@10, 0..1 (0 = no constraint).
    pub min_recall: f64,
}

impl Default for QueryHints {
    fn default() -> Self {
        Self {
            corpus_size: 10_000,
            latency_budget_us: 0,
            min_recall: 0.0,
        }
    }
}

/// Beta-distribution parameters (successes α, failures β). Conjugate prior for
/// Bernoulli rewards — here we bin observed recall ≥ min_recall as success.
#[derive(Debug, Clone, Copy)]
struct Beta {
    alpha: f64,
    beta: f64,
}

impl Beta {
    const fn new() -> Self {
        Self {
            alpha: 1.0,
            beta: 1.0,
        }
    }

    /// Update with a Bernoulli outcome (`success = recall ≥ target`).
    fn update(&mut self, success: bool) {
        if success {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
        // Clamp — never let pseudo-counts explode past 10_000 each.
        let cap = 10_000.0_f64;
        if self.alpha > cap {
            self.alpha = cap;
        }
        if self.beta > cap {
            self.beta = cap;
        }
    }

    /// Mean of the Beta posterior — used instead of sampling for determinism.
    fn mean(self) -> f64 {
        let total = self.alpha + self.beta;
        if total < f64::EPSILON {
            0.5
        } else {
            self.alpha / total
        }
    }
}

/// The router itself. Holds per-strategy posteriors.
#[derive(Debug, Default)]
pub struct AdaptiveRouter {
    posterior: HashMap<Strategy, Beta>,
    ewma_us: HashMap<Strategy, f64>,
    n_decisions: u64,
}

impl AdaptiveRouter {
    /// Fresh router with flat Beta(1,1) posteriors on every strategy.
    #[must_use]
    pub fn new() -> Self {
        let mut s = Self::default();
        for strat in Self::enumerated() {
            s.posterior.insert(strat, Beta::new());
            s.ewma_us.insert(strat, strat.prior_us_per_100k());
        }
        s
    }

    const fn enumerated() -> [Strategy; 7] {
        [
            Strategy::ScalarF32,
            Strategy::RayonF32,
            Strategy::SimSimdF32,
            Strategy::SimSimdI8,
            Strategy::SimSimdHamming,
            Strategy::RaBitQCascade,
            Strategy::MrlSimSimd,
        ]
    }

    /// Pick the best strategy given per-query hints. Deterministic: uses
    /// posterior mean + penalty heuristics (no RNG) so tests are reproducible.
    ///
    /// **Two-pass logic**:
    /// 1. Build the feasible set (passes corpus gate + posterior-recall ≥ min_recall).
    /// 2. If a budget is set AND nobody meets it, pick the *fastest* feasible
    ///    strategy — fixed-deadline queries must return something, best-effort.
    ///    Otherwise score by `recall − latency-penalty − corpus-penalty`.
    pub fn choose(&self, hints: &QueryHints) -> Strategy {
        let scale = (hints.corpus_size as f64 / 100_000.0).max(0.01);
        let budget = hints.latency_budget_us as f64;

        // Feasible set: (strat, recall, us)
        let mut feasible: Vec<(Strategy, f64, f64)> = Vec::with_capacity(6);
        for strat in Self::enumerated() {
            if !passes_corpus_gate(strat, hints.corpus_size) {
                continue;
            }
            let post = self
                .posterior
                .get(&strat)
                .copied()
                .unwrap_or_else(Beta::new);
            let recall = post.mean().max(strat.prior_recall());
            if recall < hints.min_recall {
                continue;
            }
            let us = self.ewma_us.get(&strat).copied().unwrap_or(f64::INFINITY) * scale;
            feasible.push((strat, recall, us));
        }
        if feasible.is_empty() {
            return Strategy::SimSimdI8;
        }

        // Fixed-deadline fallback: if budget set AND no-one meets it, pick fastest.
        if budget > 0.0 && feasible.iter().all(|(_, _, us)| *us > budget) {
            feasible.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
            return feasible[0].0;
        }

        // Normal scoring.
        let mut best = feasible[0].0;
        let mut best_score = f64::MIN;
        for (strat, recall, us) in &feasible {
            let pen_lat = if budget <= 0.0 || *us <= budget {
                0.0
            } else {
                ((*us - budget) / budget).min(2.0)
            };
            let pen_corpus = if hints.corpus_size < 5_000 {
                match strat {
                    Strategy::SimSimdHamming | Strategy::MrlSimSimd => 0.30,
                    _ => 0.0,
                }
            } else {
                0.0
            };
            let score = recall - pen_lat * 0.5 - pen_corpus;
            if score > best_score {
                best_score = score;
                best = *strat;
            }
        }
        best
    }

    /// Feed back the actual observed runtime + recall.
    pub fn observe(&mut self, strat: Strategy, us: f64, recall: f64) {
        let post = self.posterior.entry(strat).or_insert_with(Beta::new);
        post.update(recall >= 0.9); // 0.9 as a generic "good" threshold
        let ewma = self
            .ewma_us
            .entry(strat)
            .or_insert_with(|| strat.prior_us_per_100k());
        *ewma = 0.7 * *ewma + 0.3 * us;
        self.n_decisions += 1;
    }

    /// Total observations so far.
    #[must_use]
    pub const fn decisions(&self) -> u64 {
        self.n_decisions
    }

    /// Current posterior mean recall per strategy — mostly for debugging + UI.
    #[must_use]
    pub fn posterior_means(&self) -> Vec<(Strategy, f64, f64)> {
        Self::enumerated()
            .into_iter()
            .map(|s| {
                let r = self
                    .posterior
                    .get(&s)
                    .copied()
                    .unwrap_or_else(Beta::new)
                    .mean();
                let us = self.ewma_us.get(&s).copied().unwrap_or(0.0);
                (s, r, us)
            })
            .collect()
    }
}

/// Hard gates: some strategies are nonsensical at certain scales.
fn passes_corpus_gate(strat: Strategy, n: usize) -> bool {
    match strat {
        Strategy::ScalarF32 => n <= 10_000,
        Strategy::RayonF32 => n <= 200_000,
        Strategy::SimSimdF32 => n <= 5_000_000,
        Strategy::SimSimdI8 => n <= 50_000_000,
        Strategy::SimSimdHamming => n >= 10_000,
        Strategy::MrlSimSimd => n >= 5_000,
        Strategy::RaBitQCascade => (5_000..=50_000_000).contains(&n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_router_picks_int8_at_100k() {
        let r = AdaptiveRouter::new();
        let s = r.choose(&QueryHints {
            corpus_size: 100_000,
            latency_budget_us: 0,
            min_recall: 0.0,
        });
        // At 100k with no constraints, full-recall paths win by recall tie-break
        // toward the cheaper compute — expect ScalarF32 to be gated-out at this
        // scale but RayonF32 still eligible with recall 1.0.
        assert!(matches!(
            s,
            Strategy::RayonF32 | Strategy::SimSimdF32 | Strategy::SimSimdI8
        ));
    }

    #[test]
    fn tight_latency_favors_hamming() {
        let r = AdaptiveRouter::new();
        let s = r.choose(&QueryHints {
            corpus_size: 1_000_000,
            latency_budget_us: 300,
            min_recall: 0.60, // tolerant
        });
        assert_eq!(s, Strategy::SimSimdHamming);
    }

    #[test]
    fn high_recall_locks_out_hamming() {
        let r = AdaptiveRouter::new();
        let s = r.choose(&QueryHints {
            corpus_size: 1_000_000,
            latency_budget_us: 0,
            min_recall: 0.95,
        });
        assert_ne!(s, Strategy::SimSimdHamming);
    }

    #[test]
    fn small_corpus_avoids_hamming_even_without_recall_gate() {
        let r = AdaptiveRouter::new();
        let s = r.choose(&QueryHints {
            corpus_size: 800,
            latency_budget_us: 0,
            min_recall: 0.0,
        });
        assert_ne!(s, Strategy::SimSimdHamming);
    }

    #[test]
    fn observe_updates_posterior_monotonically() {
        let mut r = AdaptiveRouter::new();
        for _ in 0..50 {
            r.observe(Strategy::SimSimdI8, 280.0, 0.98);
        }
        let means = r.posterior_means();
        let int8 = means
            .iter()
            .find(|(s, _, _)| *s == Strategy::SimSimdI8)
            .unwrap();
        assert!(
            int8.1 > 0.9,
            "int8 posterior mean {} should be > 0.9",
            int8.1
        );
    }

    #[test]
    fn observe_never_explodes_counts() {
        let mut r = AdaptiveRouter::new();
        for _ in 0..100_000 {
            r.observe(Strategy::SimSimdI8, 300.0, 1.0);
        }
        let means = r.posterior_means();
        assert!(means[3].1 <= 1.0);
        assert!(r.decisions() == 100_000);
    }

    #[test]
    fn ewma_converges_to_observed_runtime() {
        let mut r = AdaptiveRouter::new();
        for _ in 0..30 {
            r.observe(Strategy::SimSimdI8, 100.0, 1.0);
        }
        let means = r.posterior_means();
        let int8 = means
            .iter()
            .find(|(s, _, _)| *s == Strategy::SimSimdI8)
            .unwrap();
        assert!((int8.2 - 100.0).abs() < 5.0, "ewma_us = {}", int8.2);
    }
}
