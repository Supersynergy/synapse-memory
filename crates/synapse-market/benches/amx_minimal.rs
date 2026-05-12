/// amx_minimal — Pearson corr-matrix 220×220 over 252 daily returns
///
/// Three implementations:
///   1. naive  — Rust scalar reference
///   2. wide   — f32x8 NEON via `wide` crate
///   3. cblas  — cblas_sgemm → activates AMX coprocessor on M3+/M4-Max
///
/// Correctness: all three agree to ±1e-4 on fixed seed.
/// Run: cargo bench --bench amx_minimal

use std::time::{Duration, Instant};
use wide::f32x8;

// ── cblas via Accelerate framework ────────────────────────────────────────────
#[link(name = "Accelerate", kind = "framework")]
extern "C" {
    fn cblas_sgemm(
        order: u32, transa: u32, transb: u32,
        m: i32, n: i32, k: i32,
        alpha: f32,
        a: *const f32, lda: i32,
        b: *const f32, ldb: i32,
        beta: f32,
        c: *mut f32, ldc: i32,
    );
}

const CBLAS_ROW_MAJOR: u32 = 101;
const CBLAS_NO_TRANS:  u32 = 111;
const CBLAS_TRANS:     u32 = 112;

// ── workload dimensions ───────────────────────────────────────────────────────
const TICKERS: usize = 220;
const BARS:    usize = 252;
const ITERS:   usize = 200;

// ── deterministic PRNG ────────────────────────────────────────────────────────
fn rand_returns(seed: u64) -> Vec<f32> {
    let n = TICKERS * BARS;
    let mut v = Vec::with_capacity(n);
    let mut x = seed.wrapping_add(1);
    for _ in 0..n {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        // small daily returns in [-0.05, 0.05]
        let frac = (x >> 33) as f32 / (1u64 << 31) as f32 * 0.05;
        v.push(frac);
    }
    v
}

// ── shared normalizer (zero-mean, unit-var) ───────────────────────────────────
fn normalize(data: &[f32]) -> Vec<f32> {
    let mut z = vec![0f32; TICKERS * BARS];
    for t in 0..TICKERS {
        let row = &data[t * BARS..(t + 1) * BARS];
        let mu = row.iter().sum::<f32>() / BARS as f32;
        let ss: f32 = row.iter().map(|&x| (x - mu) * (x - mu)).sum();
        let inv = 1.0 / (ss.sqrt() + 1e-8);
        let zrow = &mut z[t * BARS..(t + 1) * BARS];
        for (z, &x) in zrow.iter_mut().zip(row.iter()) {
            *z = (x - mu) * inv;
        }
    }
    z
}

// ── 1. naive scalar ───────────────────────────────────────────────────────────
fn corr_naive(z: &[f32], out: &mut [f32]) {
    for i in 0..TICKERS {
        for j in 0..TICKERS {
            let ri = &z[i * BARS..(i + 1) * BARS];
            let rj = &z[j * BARS..(j + 1) * BARS];
            let mut s = 0f32;
            for k in 0..BARS { s += ri[k] * rj[k]; }
            out[i * TICKERS + j] = s / BARS as f32;
        }
    }
}

// ── 2. wide f32x8 NEON ────────────────────────────────────────────────────────
fn corr_wide(z: &[f32], out: &mut [f32]) {
    let chunks = BARS / 8;
    for i in 0..TICKERS {
        for j in 0..TICKERS {
            let ri = &z[i * BARS..(i + 1) * BARS];
            let rj = &z[j * BARS..(j + 1) * BARS];
            let mut acc = f32x8::ZERO;
            for c in 0..chunks {
                acc += f32x8::from(&ri[c * 8..c * 8 + 8])
                     * f32x8::from(&rj[c * 8..c * 8 + 8]);
            }
            let mut s: f32 = acc.as_array_ref().iter().sum();
            for k in chunks * 8..BARS { s += ri[k] * rj[k]; }
            out[i * TICKERS + j] = s / BARS as f32;
        }
    }
}

// ── 3. cblas_sgemm  (AMX coprocessor on M3+) ─────────────────────────────────
// corr = (1/N) * Z * Z^T  — single BLAS-3 call
fn corr_cblas(z: &[f32], out: &mut [f32]) {
    unsafe {
        cblas_sgemm(
            CBLAS_ROW_MAJOR,
            CBLAS_NO_TRANS, CBLAS_TRANS,
            TICKERS as i32, TICKERS as i32, BARS as i32,
            1.0 / BARS as f32,
            z.as_ptr(), BARS as i32,
            z.as_ptr(), BARS as i32,
            0.0,
            out.as_mut_ptr(), TICKERS as i32,
        );
    }
}

// ── timing helper ─────────────────────────────────────────────────────────────
fn bench_fn<F: FnMut()>(mut f: F, iters: usize) -> Vec<Duration> {
    // warmup
    for _ in 0..10 { f(); }
    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t0 = Instant::now();
        f();
        times.push(t0.elapsed());
    }
    times
}

fn stats(times: &mut Vec<Duration>) -> (Duration, Duration, Duration) {
    times.sort_unstable();
    let p50 = times[times.len() / 2];
    let p95 = times[(times.len() * 95) / 100];
    let mean = times.iter().sum::<Duration>() / times.len() as u32;
    (p50, p95, mean)
}

fn gflops(dur: Duration) -> f64 {
    // ops: 2 * TICKERS * TICKERS * BARS  (multiply-add pairs)
    let flops = 2.0 * TICKERS as f64 * TICKERS as f64 * BARS as f64;
    flops / dur.as_secs_f64() / 1e9
}

fn fmt_dur(d: Duration) -> String {
    let us = d.as_micros();
    if us >= 1000 { format!("{:.1}ms", us as f64 / 1000.0) }
    else          { format!("{}µs", us) }
}

// ── correctness check ─────────────────────────────────────────────────────────
fn check_close(a: &[f32], b: &[f32], label: &str) {
    let max_err = a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
    assert!(max_err < 1e-4, "{label}: max_err={max_err:.2e} > 1e-4");
}

// ── main ──────────────────────────────────────────────────────────────────────
fn main() {
    let raw = rand_returns(42);
    let z   = normalize(&raw);

    let mut out_naive = vec![0f32; TICKERS * TICKERS];
    let mut out_wide  = vec![0f32; TICKERS * TICKERS];
    let mut out_cblas = vec![0f32; TICKERS * TICKERS];

    // correctness
    corr_naive(&z, &mut out_naive);
    corr_wide (&z, &mut out_wide);
    corr_cblas(&z, &mut out_cblas);
    check_close(&out_naive, &out_wide,  "wide  vs naive");
    check_close(&out_naive, &out_cblas, "cblas vs naive");
    eprintln!("correctness ✓  (max_err < 1e-4 across 220×220 = 48400 cells)");

    // bench
    let mut t_naive = bench_fn(|| corr_naive(&z, &mut out_naive), ITERS);
    let mut t_wide  = bench_fn(|| corr_wide (&z, &mut out_wide),  ITERS);
    let mut t_cblas = bench_fn(|| corr_cblas(&z, &mut out_cblas), ITERS);

    let (n_p50, n_p95, n_mean) = stats(&mut t_naive);
    let (w_p50, w_p95, w_mean) = stats(&mut t_wide);
    let (c_p50, c_p95, c_mean) = stats(&mut t_cblas);

    let header = format!(
        "\n{:<12} {:>10} {:>10} {:>10} {:>10}",
        "impl", "p50", "p95", "mean", "GFLOPS"
    );
    println!("{}", header);
    println!("{}", "-".repeat(54));
    println!("{:<12} {:>10} {:>10} {:>10} {:>10.1}",
        "naive",
        fmt_dur(n_p50), fmt_dur(n_p95), fmt_dur(n_mean),
        gflops(n_mean));
    println!("{:<12} {:>10} {:>10} {:>10} {:>10.1}",
        "wide (NEON)",
        fmt_dur(w_p50), fmt_dur(w_p95), fmt_dur(w_mean),
        gflops(w_mean));
    println!("{:<12} {:>10} {:>10} {:>10} {:>10.1}",
        "cblas (AMX)",
        fmt_dur(c_p50), fmt_dur(c_p95), fmt_dur(c_mean),
        gflops(c_mean));
    println!();
    println!("speedup cblas/naive : {:.1}×", n_mean.as_secs_f64() / c_mean.as_secs_f64());
    println!("speedup cblas/wide  : {:.1}×", w_mean.as_secs_f64() / c_mean.as_secs_f64());
    println!();
    println!("workload : Pearson corr-matrix 220×220, 252 daily-return bars");
    println!("iters    : {ITERS} (+ 10 warmup)");
    println!("env      : {}", env_info());
}

fn env_info() -> String {
    // best-effort: sysctl brand string on macOS
    if let Ok(out) = std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
    {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        "unknown".into()
    }
}
