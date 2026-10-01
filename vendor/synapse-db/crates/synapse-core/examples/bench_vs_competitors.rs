//! Synapse vs the world — head-to-head micro-bench.
//!
//! Compares four retrieval paths on the same synthetic corpus:
//! * scalar f32 baseline           (naive floor)
//! * ndarray gemv                  (proxy for numpy/Python ecosystems)
//! * SimSIMD int8 (synapse Turbo)  (shipped in synapse v2.0 "Turbo")
//! * Hamming → int8 rescore k=10   (v2.1 two-stage pipeline)
//!
//! To compare against faiss / fastembed on real models, see
//! `docs/bench_2026-04-24/external-bench.md` — those require Python and are
//! tracked separately.

use std::time::Instant;

use rayon::prelude::*;

const N: usize = 100_000;
const DIM: usize = 384;
const ITERS: usize = 15;

fn corpus(n: usize, d: usize) -> Vec<f32> {
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
        for x in row {
            *x *= inv;
        }
    }
    out
}

fn bench<F: FnMut()>(mut f: F) -> f64 {
    for _ in 0..3 {
        f();
    }
    let t = Instant::now();
    for _ in 0..ITERS {
        f();
    }
    t.elapsed().as_secs_f64() * 1e6 / ITERS as f64
}

fn main() {
    eprintln!("synapse vs ecosystem — N={N} × D={DIM}, {ITERS} iters");
    eprintln!();

    let db = corpus(N, DIM);
    let q: Vec<f32> = db[..DIM].to_vec();

    // 1. Scalar baseline (numpy-no-simd floor)
    let scalar_us = bench(|| {
        let _: Vec<f32> = db
            .par_chunks(DIM)
            .map(|row| q.iter().zip(row).map(|(a, b)| a * b).sum::<f32>())
            .collect();
    });

    // 2. ndarray gemv (BLAS call if linked)
    let ndarray_us = {
        use ndarray::{Array1, Array2};
        let mat = Array2::from_shape_vec((N, DIM), db.clone()).unwrap();
        let qv = Array1::from_vec(q.clone());
        bench(|| {
            let _ = mat.dot(&qv);
        })
    };

    #[cfg(feature = "simsimd")]
    let int8_us = {
        use synapse_core::turbo::inmem_i8_index::InMemoryI8Index;
        let rows: Vec<(i64, Vec<f32>)> = db
            .chunks(DIM)
            .enumerate()
            .map(|(i, r)| (i as i64, r.to_vec()))
            .collect();
        let idx = InMemoryI8Index::build(rows);
        bench(|| {
            let _ = idx.search(&q, 10);
        })
    };
    #[cfg(not(feature = "simsimd"))]
    let int8_us = f64::NAN;

    #[cfg(feature = "simsimd")]
    let pipeline_us = {
        use synapse_core::turbo::inmem_hamming_index::InMemoryHammingIndex;
        use synapse_core::turbo::inmem_i8_index::InMemoryI8Index;
        let rows: Vec<(i64, Vec<f32>)> = db
            .chunks(DIM)
            .enumerate()
            .map(|(i, r)| (i as i64, r.to_vec()))
            .collect();
        let hidx = InMemoryHammingIndex::build(rows.clone());
        let iidx = InMemoryI8Index::build(rows);
        // Warm the id_to_row lookup cache before timing.
        let _ = iidx.rescore(&q, &[0, 1]);
        bench(|| {
            let cands = hidx.search(&q, 80);
            let ids: Vec<i64> = cands.into_iter().map(|(id, _)| id).collect();
            let mut rescored = iidx.rescore(&q, &ids);
            rescored.truncate(10);
            let _ = rescored;
        })
    };
    #[cfg(not(feature = "simsimd"))]
    let pipeline_us = f64::NAN;

    println!("| engine                           | us/query |    QPS | vs scalar |");
    println!("|----------------------------------|---------:|-------:|----------:|");
    let print = |name: &str, us: f64| {
        let qps = 1e6 / us;
        let sx = scalar_us / us;
        println!("| {name:<32} | {us:>8.0} | {qps:>6.0} | {sx:>8.2}× |");
    };
    print("scalar f32 (numpy-no-SIMD floor)", scalar_us);
    print("ndarray gemv (BLAS proxy)", ndarray_us);
    print("SimSIMD int8 (synapse Turbo)", int8_us);
    print("Hamming → i8 rescore k=10", pipeline_us);
}
