//! AOT native-CPU bench binary for synapse-ann.
//!
//! Build (native-tuned, fat LTO, debug info for profiling):
//!   RUSTFLAGS="-C target-cpu=native" cargo build --profile release-bench \
//!     -p synapse-ann --bin ann_bench_aot --features ann-usearch,ann-batch
//!
//! Run:
//!   ./target/release-bench/ann_bench_aot [--dim 128] [--n 100000] [--q 64] [--k 10]

use synapse_ann::{AnnIndex as _, usearch_backend::UsearchIndex};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |flag: &str, default: usize| -> usize {
        args.windows(2)
            .find(|w| w[0] == flag)
            .and_then(|w| w[1].parse().ok())
            .unwrap_or(default)
    };
    let dim = get("--dim", 128);
    let n = get("--n", 100_000);
    let q = get("--q", 64);
    let k = get("--k", 10);

    println!("ann_bench_aot dim={dim} n={n} q={q} k={k}");
    println!("rayon threads: {}", rayon::current_num_threads());

    // Build index.
    let mut idx = UsearchIndex::new(dim, n).unwrap();
    let vecs: Vec<Vec<f32>> = (0..n as u64).map(|i| synthetic_vec(i, dim)).collect();
    let t0 = std::time::Instant::now();
    for (i, v) in vecs.iter().enumerate() {
        idx.insert(i as u64, v).unwrap();
    }
    println!(
        "build {n} vecs: {:.1}ms",
        t0.elapsed().as_secs_f64() * 1000.0
    );

    // Queries.
    let queries: Vec<Vec<f32>> = (0..q as u64)
        .map(|i| synthetic_vec(i * 7919, dim))
        .collect();

    // Single-query baseline.
    let t1 = std::time::Instant::now();
    let mut hits_single = 0usize;
    for qv in &queries {
        let r = idx.search(qv, k).unwrap();
        hits_single += r.len();
    }
    let single_ms = t1.elapsed().as_secs_f64() * 1000.0;
    let single_qps = q as f64 / (single_ms / 1000.0);

    // Batch (rayon parallel).
    let t2 = std::time::Instant::now();
    let batch_results = idx.search_batch(&queries, k);
    let hits_batch: usize = batch_results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .map(|r| r.len())
        .sum();
    let batch_ms = t2.elapsed().as_secs_f64() * 1000.0;
    let batch_qps = q as f64 / (batch_ms / 1000.0);

    println!("single: {single_ms:.2}ms  {single_qps:.0} QPS  hits={hits_single}");
    println!("batch:  {batch_ms:.2}ms  {batch_qps:.0} QPS  hits={hits_batch}");
    println!("speedup: {:.2}×", batch_qps / single_qps);

    // Run twice for variance.
    let t3 = std::time::Instant::now();
    let _ = idx.search_batch(&queries, k);
    let batch_ms2 = t3.elapsed().as_secs_f64() * 1000.0;
    println!(
        "batch run-2: {batch_ms2:.2}ms  speedup: {:.2}×",
        single_ms / batch_ms2
    );
}

fn synthetic_vec(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            let mix = seed
                .wrapping_mul(0x9E3779B97F4A7C15)
                .wrapping_add(i as u64)
                .wrapping_mul(0xBF58476D1CE4E5B9);
            (mix as i32 as f32) / (i32::MAX as f32)
        })
        .collect()
}
