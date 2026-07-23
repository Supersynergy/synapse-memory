//! Micro-benchmark: SimSIMD cosine vs ndarray dot-product cosine
//!
//! Setup: 100k × 384 f32 matrix, 1000 random queries, brute-force top-10.
//!
//! Run:
//! ```bash
//! cargo bench -p synapse-core --features "turbo,simsimd" \
//!   --bench simsimd_vs_ndarray
//! ```

use criterion::{Criterion, Throughput, criterion_group, criterion_main};

const N: usize = 100_000;
const DIM: usize = 384;
const N_QUERIES: usize = 1_000;
const K: usize = 10;

fn xorshift(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    ((*state & 0xFFFF) as f32) / 65536.0 - 0.5
}

fn make_normalized(n: usize, d: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    let mut out = Vec::with_capacity(n * d);
    for _ in 0..n {
        let row: Vec<f32> = (0..d).map(|_| xorshift(&mut state)).collect();
        let norm: f32 = row.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-10);
        out.extend(row.iter().map(|x| x / norm));
    }
    out
}

fn top_k_indices(sims: &[f32], k: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..sims.len()).collect();
    idx.select_nth_unstable_by(k - 1, |a, b| {
        sims[*b]
            .partial_cmp(&sims[*a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    idx[..k].to_vec()
}

// ── ndarray path ──────────────────────────────────────────────────────────────

fn ndarray_cosine_batch(query: &[f32], db: &[f32], dim: usize) -> Vec<f32> {
    use ndarray::{Array2, arr1};
    let n = db.len() / dim;
    let matrix = Array2::from_shape_vec((n, dim), db.to_vec()).unwrap();
    let q = arr1(query);
    let q_norm = q.dot(&q).sqrt().max(1e-10);
    let q_n = &q / q_norm;
    matrix.dot(&q_n).into_raw_vec()
}

fn bench_ndarray(c: &mut Criterion) {
    let db = make_normalized(N, DIM, 42);
    let queries = make_normalized(N_QUERIES, DIM, 99);
    let total_ops = N_QUERIES as u64 * N as u64;

    let mut group = c.benchmark_group("cosine_100k_384");
    group.throughput(Throughput::Elements(total_ops));
    group.sample_size(10);

    group.bench_function("ndarray", |b| {
        b.iter(|| {
            let mut top = Vec::with_capacity(N_QUERIES);
            for qi in 0..N_QUERIES {
                let q = &queries[qi * DIM..(qi + 1) * DIM];
                let sims = ndarray_cosine_batch(q, &db, DIM);
                top.push(top_k_indices(&sims, K));
            }
            criterion::black_box(top)
        })
    });

    group.finish();
}

// ── SimSIMD path ──────────────────────────────────────────────────────────────

#[cfg(feature = "simsimd")]
fn bench_simsimd(c: &mut Criterion) {
    use rayon::prelude::*;
    use synapse_core::turbo::simsimd_kernels::cos_f32_batch;

    let db = make_normalized(N, DIM, 42);
    let queries = make_normalized(N_QUERIES, DIM, 99);
    let total_ops = N_QUERIES as u64 * N as u64;

    let mut group = c.benchmark_group("cosine_100k_384");
    group.throughput(Throughput::Elements(total_ops));
    group.sample_size(10);

    // Sequential simsimd: row-by-row NEON kernel
    group.bench_function("simsimd_seq", |b| {
        b.iter(|| {
            let mut top = Vec::with_capacity(N_QUERIES);
            for qi in 0..N_QUERIES {
                let q = &queries[qi * DIM..(qi + 1) * DIM];
                let sims = cos_f32_batch(q, &db, DIM);
                top.push(top_k_indices(&sims, K));
            }
            criterion::black_box(top)
        })
    });

    // Parallel simsimd: rayon over queries
    group.bench_function("simsimd_rayon", |b| {
        b.iter(|| {
            let top: Vec<_> = (0..N_QUERIES)
                .into_par_iter()
                .map(|qi| {
                    let q = &queries[qi * DIM..(qi + 1) * DIM];
                    let sims = cos_f32_batch(q, &db, DIM);
                    top_k_indices(&sims, K)
                })
                .collect();
            criterion::black_box(top)
        })
    });

    group.finish();
}

#[cfg(feature = "simsimd")]
criterion_group!(benches, bench_ndarray, bench_simsimd);

#[cfg(not(feature = "simsimd"))]
criterion_group!(benches, bench_ndarray);

criterion_main!(benches);
