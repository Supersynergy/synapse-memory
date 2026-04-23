//! Progression bench — evolution of the Synapse hot path.
//!
//! Compares, end-to-end at 100 k × 384 doc corpus:
//!
//! | Step | What | Kernel |
//! |------|------|--------|
//! | S0   | naive scalar cos f32          | scalar loop |
//! | S1   | rayon-parallel scalar cos f32 | par_chunks |
//! | S2   | SimSIMD cos f32               | simsimd NEON |
//! | S3   | SimSIMD i8 dot (quantized)    | simsimd NEON i8 |
//! | S4   | SimSIMD hamming 1-bit         | simsimd NEON b8 |
//! | S5   | MRL 128 truncate + SimSIMD    | MRL + NEON |
//!
//! Run:
//! ```bash
//! cargo bench -p synapse-core --features "turbo,simsimd" \
//!   --bench kernels_progression
//! ```

use std::time::Instant;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rayon::prelude::*;

const N: usize = 100_000;
const DIM: usize = 384;
const MRL_K: usize = 128;

fn make_corpus_f32(n: usize, d: usize) -> Vec<f32> {
    // deterministic xorshift — avoids pulling rand into the bench
    let mut state: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let mut out = Vec::with_capacity(n * d);
    for _ in 0..(n * d) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let v = ((state as i64) as f32) / (i64::MAX as f32);
        out.push(v);
    }
    normalize_rows(&mut out, d);
    out
}

fn normalize_rows(buf: &mut [f32], d: usize) {
    for row in buf.chunks_mut(d) {
        let n: f32 = row.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
        let inv = 1.0 / n;
        for x in row {
            *x *= inv;
        }
    }
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
            if v > 0.0 {
                out[i * bpr + j / 8] |= 1 << (j % 8);
            }
        }
    }
    (out, bpr)
}

// --- kernels --------------------------------------------------------------

fn scalar_cos_single(q: &[f32], row: &[f32]) -> f32 {
    q.iter().zip(row).map(|(a, b)| a * b).sum()
}

fn s0_scalar_cos(q: &[f32], db: &[f32], d: usize) -> Vec<f32> {
    db.chunks(d).map(|r| scalar_cos_single(q, r)).collect()
}

fn s1_rayon_cos(q: &[f32], db: &[f32], d: usize) -> Vec<f32> {
    db.par_chunks(d).map(|r| scalar_cos_single(q, r)).collect()
}

#[cfg(feature = "simsimd")]
fn s2_simsimd_cos(q: &[f32], db: &[f32], d: usize) -> Vec<f32> {
    use synapse_core::turbo::simsimd_kernels::cos_f32;
    db.par_chunks(d)
        .map(|r| cos_f32(q, r).unwrap_or(0.0))
        .collect()
}

#[cfg(feature = "simsimd")]
fn s3_simsimd_dot_i8(q: &[i8], codes: &[i8], scales: &[f32], d: usize) -> Vec<f32> {
    use synapse_core::turbo::simsimd_kernels::dot_i8;
    codes
        .par_chunks(d)
        .zip(scales.par_iter())
        .map(|(r, &s)| dot_i8(q, r).map(|v| v as f32 * s).unwrap_or(0.0))
        .collect()
}

#[cfg(feature = "simsimd")]
fn s4_simsimd_hamming(q: &[u8], db: &[u8], bpr: usize) -> Vec<f64> {
    use synapse_core::turbo::simsimd_kernels::hamming_b8;
    db.par_chunks(bpr)
        .map(|r| hamming_b8(q, r).unwrap_or(0.0))
        .collect()
}

// --- criterion groups -----------------------------------------------------

fn bench_progression(c: &mut Criterion) {
    let db_f32 = make_corpus_f32(N, DIM);
    let q_f32: Vec<f32> = db_f32[..DIM].to_vec(); // query = doc-0

    let (codes, scales) = quantize_i8(&db_f32, DIM);
    let (q_codes, q_scale) = quantize_i8(&q_f32, DIM);
    let _ = q_scale; // scalar path unused

    let (bits, bpr) = binarize(&db_f32, DIM);
    let (q_bits, _bpr_q) = binarize(&q_f32, DIM);
    let q_bits_flat = q_bits[..bpr].to_vec();

    let mut g = c.benchmark_group("synapse_hot_path_100k_384");
    g.throughput(Throughput::Elements(N as u64));
    g.sample_size(20);

    g.bench_function(BenchmarkId::new("S0", "scalar_cos_f32"), |b| {
        b.iter(|| s0_scalar_cos(&q_f32, &db_f32, DIM));
    });
    g.bench_function(BenchmarkId::new("S1", "rayon_cos_f32"), |b| {
        b.iter(|| s1_rayon_cos(&q_f32, &db_f32, DIM));
    });

    #[cfg(feature = "simsimd")]
    {
        g.bench_function(BenchmarkId::new("S2", "simsimd_cos_f32"), |b| {
            b.iter(|| s2_simsimd_cos(&q_f32, &db_f32, DIM));
        });
        g.bench_function(BenchmarkId::new("S3", "simsimd_dot_i8"), |b| {
            b.iter(|| s3_simsimd_dot_i8(&q_codes, &codes, &scales, DIM));
        });
        g.bench_function(BenchmarkId::new("S4", "simsimd_hamming_b8"), |b| {
            b.iter(|| s4_simsimd_hamming(&q_bits_flat, &bits, bpr));
        });
    }

    // --- MRL 128 truncation --------------------------------------------
    #[cfg(feature = "simsimd")]
    {
        use synapse_core::matryoshka::truncate_row;
        let db_mrl: Vec<f32> = db_f32
            .chunks(DIM)
            .flat_map(|r| truncate_row(r, MRL_K))
            .collect();
        let q_mrl = truncate_row(&q_f32, MRL_K);

        g.bench_function(BenchmarkId::new("S5", "mrl128_simsimd_cos"), |b| {
            b.iter(|| s2_simsimd_cos(&q_mrl, &db_mrl, MRL_K));
        });
    }

    g.finish();
}

// --- bench runner with markdown output for chart --------------------------

fn run_and_dump_markdown() {
    let db_f32 = make_corpus_f32(N, DIM);
    let q_f32: Vec<f32> = db_f32[..DIM].to_vec();

    let (codes, scales) = quantize_i8(&db_f32, DIM);
    let (q_codes, _) = quantize_i8(&q_f32, DIM);
    let (bits, bpr) = binarize(&db_f32, DIM);
    let (q_bits, _) = binarize(&q_f32, DIM);
    let q_bits_flat = q_bits[..bpr].to_vec();

    let iters = 20;
    let mut rows: Vec<(&str, f64)> = Vec::new();

    macro_rules! time {
        ($label:expr, $block:block) => {{
            for _ in 0..3 { let _ = { $block }; }
            let t = Instant::now();
            for _ in 0..iters { let _ = { $block }; }
            let us = t.elapsed().as_secs_f64() * 1e6 / iters as f64;
            rows.push(($label, us));
        }};
    }

    time!("S0 scalar cos f32",  { s0_scalar_cos(&q_f32, &db_f32, DIM) });
    time!("S1 rayon cos f32",   { s1_rayon_cos(&q_f32, &db_f32, DIM) });

    #[cfg(feature = "simsimd")]
    {
        time!("S2 simsimd cos f32",   { s2_simsimd_cos(&q_f32, &db_f32, DIM) });
        time!("S3 simsimd dot i8",    { s3_simsimd_dot_i8(&q_codes, &codes, &scales, DIM) });
        time!("S4 simsimd hamming b8",{ s4_simsimd_hamming(&q_bits_flat, &bits, bpr) });

        use synapse_core::matryoshka::truncate_row;
        let db_mrl: Vec<f32> = db_f32.chunks(DIM).flat_map(|r| truncate_row(r, MRL_K)).collect();
        let q_mrl = truncate_row(&q_f32, MRL_K);
        time!("S5 MRL128 simsimd cos", { s2_simsimd_cos(&q_mrl, &db_mrl, MRL_K) });
    }

    eprintln!("\n## Progression chart (M4 Max, 100 k × {DIM}, 20 iters)\n");
    eprintln!("| step | kernel                 | us/query | QPS      | speed-up |");
    eprintln!("|------|------------------------|---------:|---------:|---------:|");
    let base = rows.first().map(|r| r.1).unwrap_or(1.0);
    for (label, us) in &rows {
        let qps = 1e6 / us;
        let sx = base / us;
        eprintln!("| {:<4} | {:<22} | {:>8.0} | {:>8.0} | {:>7.2}× |",
                  label.chars().take(4).collect::<String>(), label, us, qps, sx);
    }
    eprintln!();
    // ASCII bar
    let max_us = rows.iter().map(|r| r.1).fold(0_f64, f64::max);
    eprintln!("```");
    for (label, us) in &rows {
        let len = (us / max_us * 50.0) as usize;
        eprintln!("{:<28} {} {:.0} us", label, "█".repeat(len.max(1)), us);
    }
    eprintln!("```");
}

criterion_group!(benches, bench_progression);
criterion_main!(benches);

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn smoke() { run_and_dump_markdown(); }
}
