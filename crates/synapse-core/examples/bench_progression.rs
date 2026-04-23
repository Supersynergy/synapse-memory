//! Standalone progression benchmark — prints ASCII chart to stdout.
//!
//! ```bash
//! RUSTFLAGS="-C target-cpu=native" cargo run -p synapse-core --release \
//!     --features "turbo,simsimd" --example bench_progression
//! ```

use std::time::Instant;

use rayon::prelude::*;

const N: usize = 100_000;
const DIM: usize = 384;
const MRL_K: usize = 128;
const ITERS: usize = 20;

fn make_corpus(n: usize, d: usize) -> Vec<f32> {
    let mut state: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let mut out = Vec::with_capacity(n * d);
    for _ in 0..(n * d) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push((state as i64 as f32) / (i64::MAX as f32));
    }
    for row in out.chunks_mut(d) {
        let n: f32 = row.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
        let inv = 1.0 / n;
        for x in row { *x *= inv; }
    }
    out
}

fn quantize_i8(buf: &[f32], d: usize) -> (Vec<i8>, Vec<f32>) {
    let rows = buf.len() / d;
    let mut codes = vec![0_i8; buf.len()];
    let mut scales = vec![0_f32; rows];
    for (i, row) in buf.chunks(d).enumerate() {
        let amax = row.iter().fold(0_f32, |a, &v| a.max(v.abs())).max(1e-8);
        scales[i] = amax / 127.0;
        let inv = 1.0 / amax;
        for (j, &v) in row.iter().enumerate() {
            codes[i * d + j] = (v * inv * 127.0).round().clamp(-127.0, 127.0) as i8;
        }
    }
    (codes, scales)
}

fn binarize(buf: &[f32], d: usize) -> (Vec<u8>, usize) {
    let rows = buf.len() / d;
    let bpr = d.div_ceil(8);
    let mut out = vec![0_u8; rows * bpr];
    for (i, row) in buf.chunks(d).enumerate() {
        for (j, &v) in row.iter().enumerate() {
            if v > 0.0 { out[i * bpr + j / 8] |= 1 << (j % 8); }
        }
    }
    (out, bpr)
}

fn scalar_cos(q: &[f32], r: &[f32]) -> f32 {
    q.iter().zip(r).map(|(a, b)| a * b).sum()
}

fn time_fn<F: FnMut()>(mut f: F) -> f64 {
    for _ in 0..3 { f(); }
    let t = Instant::now();
    for _ in 0..ITERS { f(); }
    t.elapsed().as_secs_f64() * 1e6 / ITERS as f64
}

fn main() {
    eprintln!("building corpus N={N} dim={DIM}…");
    let db = make_corpus(N, DIM);
    let q: Vec<f32> = db[..DIM].to_vec();
    let (codes, scales) = quantize_i8(&db, DIM);
    let (q_codes, _) = quantize_i8(&q, DIM);
    let (bits, bpr) = binarize(&db, DIM);
    let (qbits, _) = binarize(&q, DIM);

    let mut rows: Vec<(&'static str, f64)> = Vec::new();

    rows.push(("S0 scalar cos f32", time_fn(|| {
        let _: Vec<f32> = db.chunks(DIM).map(|r| scalar_cos(&q, r)).collect();
    })));

    rows.push(("S1 rayon scalar cos f32", time_fn(|| {
        let _: Vec<f32> = db.par_chunks(DIM).map(|r| scalar_cos(&q, r)).collect();
    })));

    #[cfg(feature = "simsimd")]
    {
        use synapse_core::matryoshka::truncate_row;
        use synapse_core::turbo::simsimd_kernels::{cos_f32, dot_i8, hamming_b8};

        rows.push(("S2 SimSIMD cos f32", time_fn(|| {
            let _: Vec<f32> = db.par_chunks(DIM).map(|r| cos_f32(&q, r).unwrap_or(0.0)).collect();
        })));

        rows.push(("S3 SimSIMD dot i8", time_fn(|| {
            let _: Vec<f32> = codes.par_chunks(DIM).zip(scales.par_iter())
                .map(|(r, &s)| dot_i8(&q_codes, r).map(|v| v as f32 * s).unwrap_or(0.0))
                .collect();
        })));

        rows.push(("S4 SimSIMD hamming b8", time_fn(|| {
            let _: Vec<f64> = bits.par_chunks(bpr)
                .map(|r| hamming_b8(&qbits[..bpr], r).unwrap_or(0.0))
                .collect();
        })));

        let db_mrl: Vec<f32> = db.chunks(DIM).flat_map(|r| truncate_row(r, MRL_K)).collect();
        let q_mrl = truncate_row(&q, MRL_K);
        rows.push(("S5 MRL128 SimSIMD cos", time_fn(|| {
            let _: Vec<f32> = db_mrl.par_chunks(MRL_K)
                .map(|r| cos_f32(&q_mrl, r).unwrap_or(0.0))
                .collect();
        })));
    }

    // S6 — ndarray gemv (what NdArraySearch.search uses internally)
    {
        use ndarray::{Array1, Array2};
        let mat = Array2::from_shape_vec((N, DIM), db.clone()).expect("shape");
        let qv = Array1::from_vec(q.clone());
        rows.push(("S6 ndarray gemv cos", time_fn(|| {
            let _ = mat.dot(&qv);
        })));
    }

    let base = rows.first().map(|r| r.1).unwrap_or(1.0);
    let max = rows.iter().map(|r| r.1).fold(0_f64, f64::max);

    println!("\n## Synapse Hot-Path Progression — M4 Max, N={N} × D={DIM}, {ITERS} iters\n");
    println!("| step | kernel                     | us/query |     QPS | speed-up |");
    println!("|------|----------------------------|---------:|--------:|---------:|");
    for (label, us) in &rows {
        let qps = 1e6 / us;
        let sx = base / us;
        let tag = &label[..2];
        println!("| {tag:<4} | {name:<26} | {us:>8.0} | {qps:>7.0} | {sx:>7.2}× |",
                 name = &label[3..]);
    }

    println!("\n```");
    for (label, us) in &rows {
        let len = (us / max * 55.0) as usize;
        println!("{label:<28} {bar} {us:>6.0} us",
                 bar = "█".repeat(len.max(1)));
    }
    println!("```\n");

    println!("baseline S0 = {base:.0} us · fastest = {:.0} us · total speed-up = {:.1}×",
             rows.last().map(|r| r.1).unwrap_or(base),
             base / rows.last().map(|r| r.1).unwrap_or(base));
}
