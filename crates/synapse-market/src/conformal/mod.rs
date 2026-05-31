//! Split-conformal prediction intervals.
//!
//! Usage:
//!   let cp = Conformal::fit_split(&scores, &y, |i| model_predict(i));
//!   let (lo, hi) = cp.interval(new_pred);
//!   let cov = cp.coverage(&y_test, &preds_test);

/// Split-conformal prediction wrapper.
pub struct Conformal {
    pub alpha: f32,
    pub calibration: Vec<f32>,
    /// Sorted (1-alpha) quantile of non-conformity scores.
    pub q_hat: f32,
}

impl Conformal {
    /// Fit on calibration set.
    ///
    /// `scores[i]` = non-conformity score for calibration point `i`.
    /// `y` is unused in the pure split-CP path but kept for API symmetry.
    /// `predict` is also kept for API symmetry (F: Fn(usize) -> f32).
    pub fn fit_split<F>(scores: &[f32], _y: &[f32], _predict: F) -> Self
    where
        F: Fn(usize) -> f32,
    {
        let alpha = 0.20_f32; // 80% coverage
        let q_hat = quantile_upper(scores, 1.0 - alpha);
        Self {
            alpha,
            calibration: scores.to_vec(),
            q_hat,
        }
    }

    /// Fit with explicit alpha.
    pub fn fit_split_alpha<F>(scores: &[f32], _y: &[f32], alpha: f32, _predict: F) -> Self
    where
        F: Fn(usize) -> f32,
    {
        let q_hat = quantile_upper(scores, 1.0 - alpha);
        Self {
            alpha,
            calibration: scores.to_vec(),
            q_hat,
        }
    }

    /// Return (lower, upper) prediction interval for a point prediction.
    pub fn interval(&self, prediction: f32) -> (f32, f32) {
        (prediction - self.q_hat, prediction + self.q_hat)
    }

    /// Empirical coverage: fraction of y_test[i] inside interval(pred_test[i]).
    pub fn coverage(&self, y_test: &[f32], pred_test: &[f32]) -> f64 {
        let n = y_test.len().min(pred_test.len());
        if n == 0 {
            return 0.0;
        }
        let covered = (0..n)
            .filter(|&i| {
                let (lo, hi) = self.interval(pred_test[i]);
                y_test[i] >= lo && y_test[i] <= hi
            })
            .count();
        covered as f64 / n as f64
    }
}

/// Upper (1-alpha) quantile of `scores` using the inflate-by-1 correction.
///
/// Returns q such that at least ceil((n+1)*(1-alpha))/n fraction covered.
fn quantile_upper(scores: &[f32], level: f32) -> f32 {
    if scores.is_empty() {
        return f32::INFINITY;
    }
    let mut sorted = scores.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = sorted.len();
    // Inflate: index = ceil((n+1) * level) - 1, clamped.
    let idx_f = ((n + 1) as f32 * level).ceil() as usize;
    let idx = idx_f.saturating_sub(1).min(n - 1);
    sorted[idx]
}
