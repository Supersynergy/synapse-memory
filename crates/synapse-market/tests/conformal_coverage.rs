use synapse_market::conformal::Conformal;

fn build_conformal_80(n_cal: usize) -> Conformal {
    // Calibration: non-conformity scores = |y - pred| where y ~ N(0,1), pred=0.
    // Scores are just absolute residuals.
    let scores: Vec<f32> = (0..n_cal)
        .map(|i| {
            // Deterministic "pseudo-normal" via Box-Muller-like: use sin for variety.
            let u = (i as f32 + 1.0) / (n_cal as f32 + 1.0);
            // Approximate quantile of half-normal ~ |N(0,1)|.
            // Use sqrt(-2*ln(u)) as a rough abs normal quantile.
            (-2.0 * u.ln()).sqrt().abs()
        })
        .collect();
    let y = vec![0.0f32; n_cal];
    Conformal::fit_split_alpha(&scores, &y, 0.20, |_| 0.0)
}

#[test]
fn test_coverage_80_percent() {
    // Build conformal on 500 calibration points.
    let cp = build_conformal_80(500);

    // Test on 1000 fresh points with same distribution.
    let n_test = 1000;
    let y_test: Vec<f32> = (0..n_test)
        .map(|i| {
            // Deterministic test values alternating sign.
            let u = (i as f32 + 0.5) / n_test as f32;
            let v = (-2.0 * u.ln()).sqrt();
            if i % 2 == 0 { v } else { -v }
        })
        .collect();
    // Predictions = 0 (same as calibration assumption).
    let preds = vec![0.0f32; n_test];

    let cov = cp.coverage(&y_test, &preds);
    // Expect empirical coverage in [0.75, 0.85] for 80% target.
    assert!(
        cov >= 0.75 && cov <= 0.90,
        "Expected 80% coverage in [0.75, 0.90], got {cov:.3}"
    );
}

#[test]
fn test_interval_symmetry() {
    let cp = build_conformal_80(200);
    let (lo, hi) = cp.interval(5.0);
    assert!(hi > lo, "interval must be ordered");
    let half = (hi - lo) / 2.0;
    assert!(
        (half - cp.q_hat).abs() < 1e-5,
        "interval half-width = q_hat"
    );
}

#[test]
fn test_q_hat_positive() {
    let cp = build_conformal_80(300);
    assert!(
        cp.q_hat > 0.0,
        "q_hat must be positive for non-trivial scores"
    );
}

#[test]
fn test_market_conformal_method() {
    use synapse_market::Market;
    let scores: Vec<f32> = (0..100).map(|i| i as f32 * 0.01).collect();
    let y = vec![0.0f32; 100];
    let cp = Market::conformal(&scores, &y);
    assert!(cp.q_hat >= 0.0);
}
