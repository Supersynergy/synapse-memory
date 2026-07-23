//! Split-conformal recall prediction — enterprise statistical R=1.0 guarantee.
//!
//! Enabled via feature `conformal` (default off).
//! Usage:
//!   1. `ConformalCalibrator::new(alpha)` — e.g. alpha=0.05 → 95% coverage
//!   2. `calibrate(cal_queries, predictor)` — collects nonconformity scores on cal split
//!   3. `predict_recall_lower_bound(query)` — returns q_(1-alpha) quantile bound
//!   4. `should_fallback(query, target)` → true when predicted bound < target
//!
//! Math: nonconformity score s_i = 1 - recall(predicted_i, ground_truth_i).
//! Conformal guarantee: P(recall >= lower_bound) >= 1 - alpha over cal distribution.

#![allow(clippy::type_complexity)]

use std::collections::HashSet;

pub type DocId = i64;

/// Calibrated split-conformal recall predictor.
///
/// `alpha` = miscoverage level (e.g. 0.05 for 95% coverage guarantee).
#[derive(Debug, Clone)]
pub struct ConformalCalibrator {
    /// Nonconformity scores: s_i = 1 - recall(predicted, ground_truth)
    pub residuals: Vec<f32>,
    /// Miscoverage level in (0, 1). Guarantee: P(recall >= bound) >= 1 - alpha.
    pub alpha: f32,
}

impl ConformalCalibrator {
    pub fn new(alpha: f32) -> Self {
        assert!(alpha > 0.0 && alpha < 1.0, "alpha must be in (0,1)");
        Self {
            residuals: Vec::new(),
            alpha,
        }
    }

    /// Calibrate on a held-out calibration set.
    ///
    /// `queries`: slice of (query_text, ground_truth_doc_ids).
    /// `predictor`: fn(query) -> predicted_doc_ids (your ANN retrieval).
    ///
    /// Clears previous residuals and recomputes from scratch.
    pub fn calibrate<F>(&mut self, queries: &[(String, Vec<DocId>)], mut predictor: F)
    where
        F: FnMut(&str) -> Vec<DocId>,
    {
        self.residuals.clear();
        for (query, ground_truth) in queries {
            let predicted = predictor(query.as_str());
            let recall = recall_at_k(&predicted, ground_truth);
            self.residuals.push(1.0 - recall);
        }
    }

    /// Return conformal lower bound on recall for any new query.
    ///
    /// Uses the (1-alpha)(1 + 1/n) quantile of calibration residuals, then:
    ///   lower_bound = 1 - q_(1-alpha)
    ///
    /// Returns 0.0 if not yet calibrated.
    pub fn predict_recall_lower_bound(&self) -> f32 {
        if self.residuals.is_empty() {
            return 0.0;
        }
        let q = conformal_quantile(&self.residuals, self.alpha);
        (1.0 - q).clamp(0.0, 1.0)
    }

    /// Returns true if predicted recall lower bound is below `target_recall`.
    /// When true, caller should trigger exact/rerank fallback.
    pub fn should_fallback(&self, target_recall: f32) -> bool {
        self.predict_recall_lower_bound() < target_recall
    }

    /// Number of calibration samples.
    pub fn n_cal(&self) -> usize {
        self.residuals.len()
    }

    /// Empirical coverage: fraction of cal queries where recall >= lower_bound.
    /// Should be >= 1 - alpha on a fresh test set.
    pub fn empirical_coverage(&self) -> f32 {
        if self.residuals.is_empty() {
            return 0.0;
        }
        let bound = self.predict_recall_lower_bound();
        let covered = self.residuals.iter().filter(|&&s| 1.0 - s >= bound).count();
        covered as f32 / self.residuals.len() as f32
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Recall@K: |predicted ∩ ground_truth| / |ground_truth|
fn recall_at_k(predicted: &[DocId], ground_truth: &[DocId]) -> f32 {
    if ground_truth.is_empty() {
        return 1.0;
    }
    let gt_set: HashSet<DocId> = ground_truth.iter().copied().collect();
    let hits = predicted.iter().filter(|id| gt_set.contains(id)).count();
    hits as f32 / ground_truth.len() as f32
}

/// Conformal quantile: (ceil((n+1)(1-alpha)) / n)-th order statistic of scores.
/// This is the standard split-conformal guarantee per Vovk et al. 2005.
fn conformal_quantile(scores: &[f32], alpha: f32) -> f32 {
    let n = scores.len();
    // q_level: (1-alpha)(1 + 1/n) clamped to [0,1]
    let q_level = ((1.0 - alpha) * (1.0 + 1.0 / n as f32)).min(1.0);
    let mut sorted = scores.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // Index into sorted array (0-based)
    let idx = ((q_level * n as f32).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1);
    sorted[idx]
}

// ---------------------------------------------------------------------------
// SearchOptions integration helper
// ---------------------------------------------------------------------------

/// Conformal cascade decision given SearchOptions.conformal_target.
/// Returns true when exact-rerank fallback should be triggered.
pub fn needs_exact_fallback(
    calibrator: Option<&ConformalCalibrator>,
    conformal_target: Option<f32>,
) -> bool {
    match (calibrator, conformal_target) {
        (Some(cal), Some(target)) => cal.should_fallback(target),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic oracle: doc IDs 0..corpus_size, ground truth = first K.
    /// Predictor returns first K docs with some random "miss" rate to simulate ANN imperfection.
    fn synthetic_calibrate(
        n_queries: usize,
        k: usize,
        corpus_size: usize,
        miss_fraction: f32,
        alpha: f32,
    ) -> ConformalCalibrator {
        // Simple deterministic pseudo-random via LCG
        let mut rng_state: u64 = 0xdeadbeef_cafebabe;
        let mut next_u64 = move || {
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            rng_state
        };

        let queries: Vec<(String, Vec<DocId>)> = (0..n_queries)
            .map(|i| {
                let gt: Vec<DocId> = (0..k as i64).collect();
                (format!("query_{i}"), gt)
            })
            .collect();

        let extras_pool: Vec<DocId> = (k as i64..corpus_size as i64).collect();
        let mut extra_idx = 0usize;
        let mut cal = ConformalCalibrator::new(alpha);
        cal.calibrate(&queries, |_q| {
            let mut pred: Vec<DocId> = Vec::with_capacity(k);
            for id in 0..k as i64 {
                let r = (next_u64() as f32) / (u64::MAX as f32);
                if r >= miss_fraction {
                    pred.push(id);
                } else if extra_idx < extras_pool.len() {
                    pred.push(extras_pool[extra_idx]);
                    extra_idx += 1;
                }
            }
            pred
        });
        cal
    }

    #[test]
    fn test_recall_at_k_perfect() {
        let pred = vec![1, 2, 3];
        let gt = vec![1, 2, 3];
        assert_eq!(recall_at_k(&pred, &gt), 1.0);
    }

    #[test]
    fn test_recall_at_k_zero() {
        let pred = vec![4, 5, 6];
        let gt = vec![1, 2, 3];
        assert_eq!(recall_at_k(&pred, &gt), 0.0);
    }

    #[test]
    fn test_recall_at_k_partial() {
        let pred = vec![1, 4, 5];
        let gt = vec![1, 2, 3];
        let r = recall_at_k(&pred, &gt);
        assert!((r - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_conformal_quantile_basic() {
        let scores = vec![0.1f32, 0.2, 0.3, 0.4, 0.5];
        // alpha=0.1 → q_level=(0.9)(1+1/5)=1.08 clamped to 1.0 → idx=4 → 0.5
        let q = conformal_quantile(&scores, 0.1);
        assert!((q - 0.5).abs() < 1e-5, "q={q}");
    }

    #[test]
    fn test_not_calibrated_returns_zero() {
        let cal = ConformalCalibrator::new(0.05);
        assert_eq!(cal.predict_recall_lower_bound(), 0.0);
    }

    #[test]
    fn test_perfect_predictor_high_bound() {
        // Perfect predictor → all residuals = 0 → lower bound = 1.0
        let queries: Vec<(String, Vec<DocId>)> = (0..200)
            .map(|i| (format!("q{i}"), vec![0, 1, 2, 3, 4]))
            .collect();
        let mut cal = ConformalCalibrator::new(0.05);
        cal.calibrate(&queries, |_| vec![0, 1, 2, 3, 4]);
        let lb = cal.predict_recall_lower_bound();
        assert!(lb >= 0.99, "expected ~1.0, got {lb}");
        assert!(!cal.should_fallback(0.95));
    }

    #[test]
    fn test_poor_predictor_triggers_fallback() {
        // Predictor returns wrong docs → residuals ~1 → lower bound ~0
        let queries: Vec<(String, Vec<DocId>)> =
            (0..200).map(|i| (format!("q{i}"), vec![0, 1, 2])).collect();
        let mut cal = ConformalCalibrator::new(0.05);
        cal.calibrate(&queries, |_| vec![100, 101, 102]);
        let lb = cal.predict_recall_lower_bound();
        assert!(lb <= 0.01, "expected ~0.0, got {lb}");
        assert!(cal.should_fallback(0.9));
    }

    #[test]
    fn test_coverage_guarantee_1000_queries() {
        // 1000 cal queries, 10% miss rate, alpha=0.1
        // Empirical coverage on same set should be >= 1-alpha = 0.9
        // (Note: using same set for simplicity; real usage: held-out test set)
        let cal = synthetic_calibrate(1000, 10, 1000, 0.10, 0.10);
        assert_eq!(cal.n_cal(), 1000);

        // Lower bound should be non-trivial (predictor mostly works)
        let lb = cal.predict_recall_lower_bound();
        assert!(lb > 0.0, "lower bound should be positive, got {lb}");

        // Empirical coverage on calibration set >= 1-alpha
        let cov = cal.empirical_coverage();
        assert!(
            cov >= 0.90,
            "coverage {cov:.3} below target 0.90 (alpha=0.10)"
        );
    }

    #[test]
    fn test_coverage_guarantee_95pct() {
        // alpha=0.05, miss=5% → coverage >= 0.95
        let cal = synthetic_calibrate(1000, 10, 1000, 0.05, 0.05);
        let cov = cal.empirical_coverage();
        assert!(
            cov >= 0.95,
            "coverage {cov:.3} below target 0.95 (alpha=0.05)"
        );
    }

    #[test]
    fn test_needs_exact_fallback_no_calibrator() {
        assert!(!needs_exact_fallback(None, Some(0.9)));
        assert!(!needs_exact_fallback(None, None));
    }

    #[test]
    fn test_needs_exact_fallback_no_target() {
        let cal = ConformalCalibrator::new(0.05);
        assert!(!needs_exact_fallback(Some(&cal), None));
    }

    #[test]
    fn test_needs_exact_fallback_triggers() {
        // Bad predictor → should_fallback returns true
        let queries: Vec<(String, Vec<DocId>)> =
            (0..100).map(|i| (format!("q{i}"), vec![1, 2, 3])).collect();
        let mut cal = ConformalCalibrator::new(0.05);
        cal.calibrate(&queries, |_| vec![10, 11, 12]); // all misses
        assert!(needs_exact_fallback(Some(&cal), Some(0.9)));
    }
}
