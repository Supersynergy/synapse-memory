/// Integration test: Market::correlation_matrix vs naive O(n²) Welford.
use synapse_market::Market;

fn rand_series(seed: u64, n: usize) -> Vec<f32> {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((x >> 33) as f32) / (u32::MAX as f32) * 200.0 + 10.0 // price-like positive
        })
        .collect()
}

fn naive_pearson(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n < 2 {
        return 0.0;
    }
    let mean_a = a[..n].iter().sum::<f32>() / n as f32;
    let mean_b = b[..n].iter().sum::<f32>() / n as f32;
    let (mut num, mut da2, mut db2) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..n {
        let da = a[i] - mean_a;
        let db = b[i] - mean_b;
        num += da * db;
        da2 += da * da;
        db2 += db * db;
    }
    let denom = (da2 * db2).sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        num / denom
    }
}

#[test]
fn corr_matrix_agrees_with_naive() {
    const TICKERS: usize = 220;
    const DAYS: usize = 252;

    let market = Market::open_in_memory().unwrap();

    // Generate deterministic price series and ingest
    let names: Vec<String> = (0..TICKERS).map(|i| format!("T{i:03}")).collect();
    let closes: Vec<Vec<f32>> = (0..TICKERS)
        .map(|i| rand_series(i as u64 * 7919 + 42, DAYS))
        .collect();

    for (i, name) in names.iter().enumerate() {
        let rows: Vec<(i64, f64, f64, f64, f64, f64)> = (0..DAYS)
            .map(|d| {
                let c = closes[i][d] as f64;
                (d as i64, c, c * 1.001, c * 0.999, c, 1000.0)
            })
            .collect();
        market.ingest_ohlcv(name, &rows).unwrap();
    }

    let ticker_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let cm = market
        .correlation_matrix(&ticker_refs, 0..DAYS as i64)
        .unwrap();

    assert_eq!(cm.n, TICKERS);
    assert_eq!(cm.data.len(), TICKERS * TICKERS);

    // Check diagonal = 1
    for i in 0..TICKERS {
        assert!(
            (cm.get(i, i) - 1.0).abs() < 1e-4,
            "diagonal[{i}] = {}",
            cm.get(i, i)
        );
    }

    // Check symmetry
    for i in 0..TICKERS {
        for j in 0..TICKERS {
            assert!(
                (cm.get(i, j) - cm.get(j, i)).abs() < 1e-4,
                "asymmetric at ({i},{j})"
            );
        }
    }

    // Sample 20 pairs vs naive
    let pairs: Vec<(usize, usize)> = (0..20)
        .map(|k| (k * 11 % TICKERS, k * 17 % TICKERS))
        .collect();
    for (i, j) in pairs {
        let expected = naive_pearson(&closes[i][..DAYS], &closes[j][..DAYS]);
        let got = cm.get(i, j);
        assert!(
            (got - expected).abs() < 1e-3,
            "pair ({i},{j}): got {got:.6} expected {expected:.6}"
        );
    }
}
