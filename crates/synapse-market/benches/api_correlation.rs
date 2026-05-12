/// api_correlation — 220 tickers × 252d, AMX-auto vs NEON fallback
///
/// Run: cargo bench --bench api_correlation
use std::time::Instant;
use synapse_market::analytics::{correlation_matrix_amx, neon::correlation_f32};

fn rand_series(seed: u64, n: usize) -> Vec<f32> {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (0..n).map(|_| {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((x >> 33) as f32) / (u32::MAX as f32) - 0.5
    }).collect()
}

fn build_mat(tickers: usize, days: usize) -> Vec<f32> {
    let mut mat = vec![0.0f32; tickers * days];
    for c in 0..tickers {
        let s = rand_series(c as u64 * 31337 + 1, days);
        for r in 0..days { mat[r * tickers + c] = s[r]; }
    }
    mat
}

fn neon_corr_matrix(mat: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; cols * cols];
    for i in 0..cols {
        for j in i..cols {
            let a: Vec<f32> = (0..rows).map(|r| mat[r * cols + i]).collect();
            let b: Vec<f32> = (0..rows).map(|r| mat[r * cols + j]).collect();
            let r = correlation_f32(&a, &b);
            out[i * cols + j] = r;
            out[j * cols + i] = r;
        }
        out[i * cols + i] = 1.0;
    }
    out
}

fn main() {
    const TICKERS: usize = 220;
    const DAYS: usize = 252;
    const ITERS: usize = 100;

    let mat = build_mat(TICKERS, DAYS);

    // Warmup
    let _ = correlation_matrix_amx(&mat, DAYS, TICKERS);
    let _ = neon_corr_matrix(&mat, DAYS, TICKERS);

    // Bench AMX path
    let t0 = Instant::now();
    for _ in 0..ITERS {
        std::hint::black_box(correlation_matrix_amx(&mat, DAYS, TICKERS));
    }
    let amx_ms = t0.elapsed().as_secs_f64() * 1000.0 / ITERS as f64;

    // Bench NEON fallback path
    let t1 = Instant::now();
    for _ in 0..ITERS {
        std::hint::black_box(neon_corr_matrix(&mat, DAYS, TICKERS));
    }
    let neon_ms = t1.elapsed().as_secs_f64() * 1000.0 / ITERS as f64;

    let speedup = neon_ms / amx_ms;
    println!("220×252  AMX-auto: {amx_ms:.3}ms/iter  NEON: {neon_ms:.3}ms/iter  speedup: {speedup:.1}×");
    assert!(
        speedup >= 20.0,
        "Expected ≥20× AMX speedup, got {speedup:.1}× (AMX {amx_ms:.3}ms vs NEON {neon_ms:.3}ms)"
    );
}
