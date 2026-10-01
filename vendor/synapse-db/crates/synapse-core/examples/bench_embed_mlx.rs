//! Bench: MLX Metal embedder vs fastembed CPU.
//!
//! Run:
//! ```
//! SYNAPSE_MLX_PYTHON=~/.venvs/agents/bin/python \
//!   cargo run --release --example bench_embed_mlx \
//!     --features "embed-mlx embed turbo"
//! ```
//!
//! Reports p50/p95/mean for single-doc and batched (1/4/8/16/32/64) paths,
//! plus a 100-doc total to compare end-to-end ingest speed.

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::time::Instant;

use synapse_core::embedder_trait::TextEmbedder;

fn percentile(mut xs: Vec<f64>, p: f64) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((xs.len() as f64 - 1.0) * p).round() as usize;
    xs[i]
}

fn bench(label: &str, e: &dyn TextEmbedder, texts: &[String], iters: usize) {
    // Warmup
    let _ = e
        .embed_batch(&texts[..1.min(texts.len())].to_vec())
        .unwrap();

    // Single-doc latency
    let mut singles = Vec::with_capacity(iters);
    for i in 0..iters {
        let t = &texts[i % texts.len()];
        let t0 = Instant::now();
        let _ = e.embed_one(t).unwrap();
        singles.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let p50 = percentile(singles.clone(), 0.50);
    let p95 = percentile(singles.clone(), 0.95);
    let mean = singles.iter().sum::<f64>() / singles.len() as f64;
    println!(
        "{label:24}  single  p50={p50:6.2}ms  p95={p95:6.2}ms  mean={mean:6.2}ms  iters={iters}",
    );

    // Batched
    for &bs in &[1usize, 4, 8, 16, 32, 64] {
        if bs > texts.len() {
            continue;
        }
        let mut totals = Vec::with_capacity(20);
        for _ in 0..20 {
            let t0 = Instant::now();
            let _ = e.embed_batch(&texts[..bs].to_vec()).unwrap();
            totals.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        let p50 = percentile(totals.clone(), 0.50);
        let per = p50 / bs as f64;
        println!("{label:24}  batch={bs:3}  p50={p50:7.2}ms  per-doc={per:6.3}ms",);
    }
}

fn main() {
    let texts: Vec<String> = (0..100)
        .map(|i| {
            format!("the quick brown fox number {i} jumps over the lazy dog and runs to the river")
        })
        .collect();

    println!("\n=== synapse embed bench (M4 Max, 100 docs) ===\n");

    // fastembed baseline
    #[cfg(feature = "embed")]
    {
        let e = synapse_core::embed::Embedder::new().expect("fastembed");
        bench("fastembed-cpu", &e, &texts, 100);
    }

    // MLX sidecar
    #[cfg(feature = "embed-mlx")]
    {
        match synapse_core::embed_mlx::MlxMetalEmbedder::new() {
            Ok(e) => bench("mlx-metal-bf16", &e, &texts, 100),
            Err(e) => eprintln!("mlx skipped: {e}"),
        }
    }
}
