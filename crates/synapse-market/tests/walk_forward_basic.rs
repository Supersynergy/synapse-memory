use synapse_market::backtest_wf::{
    Bar, TradeResult, Verdict, WalkForward, deflated_sharpe_ratio, norm_cdf,
};

/// Deterministic strategy: always return +1% trade per bar.
fn always_win(bars: &[Bar]) -> Vec<TradeResult> {
    bars.iter().map(|_| TradeResult { ret: 0.01 }).collect()
}

/// Deterministic strategy: lossy — always -0.5%.
fn always_lose(bars: &[Bar]) -> Vec<TradeResult> {
    bars.iter().map(|_| TradeResult { ret: -0.005 }).collect()
}

#[test]
fn test_dsr_paper_formula_sanity() {
    // Paper example: SR=1.0, T=250, skew=0, kurt=3 (normal), N=1 trial.
    // Expected: DSR close to Phi((1.0 - 0) * sqrt(249)) since E[SR_max] ≈ 0 for N=1.
    let sr = 1.0_f64;
    let t = 250.0_f64;
    let skew = 0.0_f64;
    let kurt = 3.0_f64; // normal dist
    let dsr = deflated_sharpe_ratio(sr, t, skew, kurt, 1);
    // For N=1, E[SR_max] ~ 0, denom_sq = 1 - 0*1 + (3-1)/4*1 = 1.5
    // z = (1.0 - 0) * sqrt(249) / sqrt(1.5) ≈ 15.76 / 1.225 ≈ 12.9 → Phi ≈ 1.0
    assert!(
        dsr > 0.99,
        "DSR for sharp clean signal should approach 1.0, got {dsr}"
    );
}

#[test]
fn test_dsr_weak_signal() {
    // Very low SR, many trials → DSR < 0.5 (below chance)
    let dsr = deflated_sharpe_ratio(0.1, 100.0, 0.0, 3.0, 20);
    assert!(
        dsr < 0.5,
        "Weak SR with 20 trials should deflate below 0.5, got {dsr}"
    );
}

#[test]
fn test_norm_cdf_values() {
    // Basic sanity checks.
    let mid = norm_cdf(0.0);
    assert!((mid - 0.5).abs() < 1e-6, "Phi(0) should be 0.5, got {mid}");
    let hi = norm_cdf(4.0);
    assert!(hi > 0.99, "Phi(4.0) > 0.99, got {hi}");
    let lo = norm_cdf(-4.0);
    assert!(lo < 0.01, "Phi(-4.0) < 0.01, got {lo}");
}

#[test]
fn test_walk_forward_winning_strategy() {
    let wf = WalkForward::new(5);
    // 5000-bar range, always-winning strategy.
    let report = wf.run(0..5000, always_win);
    assert_eq!(report.folds.len(), 5);
    for fold in &report.folds {
        assert!(fold.n_trades > 0);
        assert!(
            fold.hit_rate > 0.9,
            "always_win hit_rate should be ~1.0, got {}",
            fold.hit_rate
        );
        assert!(
            fold.sharpe > 0.0,
            "sharpe should be positive for always-win"
        );
    }
}

#[test]
fn test_walk_forward_losing_strategy_overfit_or_fragile() {
    let wf = WalkForward::new(5);
    let report = wf.run(0..5000, always_lose);
    // Losing strategy should NOT be ROBUST.
    assert_ne!(
        report.verdict,
        Verdict::Robust,
        "losing strategy must not be ROBUST"
    );
}

#[test]
fn test_pbo_range() {
    let wf = WalkForward::new(5);
    let report = wf.run(0..5000, always_win);
    assert!(
        report.pbo >= 0.0 && report.pbo <= 1.0,
        "PBO must be in [0,1]"
    );
}

#[test]
fn test_market_walk_forward_method() {
    use synapse_market::Market;
    let market = Market::open_in_memory().unwrap();
    let report = market.walk_forward(0..2000, 3, always_win);
    assert_eq!(report.folds.len(), 3);
}
