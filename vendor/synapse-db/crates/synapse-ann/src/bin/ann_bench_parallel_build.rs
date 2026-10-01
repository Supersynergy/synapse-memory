//! Parallel HNSW build bench — serial vs parallel insert.
//!
//! Build:
//!   RUSTFLAGS="-C target-cpu=native" cargo build --profile release-bench \
//!     -p synapse-ann --bin ann_bench_parallel_build \
//!     --features ann-usearch,ann-parallel-build
//!
//! Run:
//!   ./target/release-bench/ann_bench_parallel_build [--dim 128] [--n 1000000]

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
    let n = get("--n", 1_000_000);

    println!("ann_bench_parallel_build dim={dim} n={n}");
    println!("rayon threads: {}", rayon::current_num_threads());

    let vecs: Vec<Vec<f32>> = (0..n as u64).map(|i| synth(i, dim)).collect();
    let ids: Vec<u64> = (0..n as u64).collect();

    // --- serial ---
    let mut idx_serial = UsearchIndex::new(dim, n).unwrap();
    let t0 = std::time::Instant::now();
    for (i, v) in vecs.iter().enumerate() {
        idx_serial.insert(i as u64, v).unwrap();
    }
    let serial_s = t0.elapsed().as_secs_f64();
    println!(
        "serial  build {n}: {serial_s:.2}s  ({:.0} vec/s)",
        n as f64 / serial_s
    );

    // --- parallel ---
    let mut idx_par = UsearchIndex::new(dim, n).unwrap();
    let t1 = std::time::Instant::now();
    let inserted = idx_par.add_batch_parallel(&ids, &vecs).unwrap();
    let par_s = t1.elapsed().as_secs_f64();
    println!(
        "parallel build {n}: {par_s:.2}s  ({:.0} vec/s)  inserted={inserted}",
        n as f64 / par_s
    );
    println!("speedup: {:.2}×", serial_s / par_s);

    // run parallel twice for variance
    let mut idx_par2 = UsearchIndex::new(dim, n).unwrap();
    let t2 = std::time::Instant::now();
    idx_par2.add_batch_parallel(&ids, &vecs).unwrap();
    let par_s2 = t2.elapsed().as_secs_f64();
    println!(
        "parallel run-2: {par_s2:.2}s  speedup: {:.2}×",
        serial_s / par_s2
    );
    println!(
        "median speedup: {:.2}×",
        serial_s / ((par_s + par_s2) / 2.0)
    );
}

fn synth(seed: u64, dim: usize) -> Vec<f32> {
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
