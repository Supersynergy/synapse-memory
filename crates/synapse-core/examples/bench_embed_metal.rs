//! Bench: Candle-Metal BGE-small vs fastembed ONNX-CPU.
//!
//! Run:
//! ```
//! cargo run --release --example bench_embed_metal \
//!   --features "embed-metal embed turbo"
//! ```
//!
//! Reports p50/p95/mean for single-doc and batched (1/4/8/16/32/64/100) paths.
//! Also validates R@5 parity: cosine similarity between Metal and ONNX outputs.

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::time::Instant;
use synapse_core::{
    embedder_trait::TextEmbedder, turbo::candle_metal_embedder::CandleMetalEmbedder,
};

#[cfg(any(feature = "embed", feature = "embed-dynamic"))]
use synapse_core::embed::Embedder as FastEmbedder;

fn percentile(mut xs: Vec<f64>, p: f64) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((xs.len() as f64 - 1.0) * p).round() as usize;
    xs[i]
}

fn bench_embedder(label: &str, e: &dyn TextEmbedder, texts: &[String], iters: usize) {
    // Warmup
    let _ = e.embed_batch(&texts[..1.min(texts.len())].to_vec());

    // Single-doc
    let mut singles = Vec::with_capacity(iters);
    for i in 0..iters {
        let t = &texts[i % texts.len()];
        let t0 = Instant::now();
        let _ = e.embed_one(t);
        singles.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let p50 = percentile(singles.clone(), 0.50);
    let p95 = percentile(singles.clone(), 0.95);
    let mean = singles.iter().sum::<f64>() / singles.len() as f64;
    println!("{label:28}  single  p50={p50:6.2}ms  p95={p95:6.2}ms  mean={mean:6.2}ms");

    // Batched
    for &bs in &[1usize, 4, 8, 16, 32, 64, 100] {
        if bs > texts.len() {
            continue;
        }
        let batch = texts[..bs].to_vec();
        let mut totals = Vec::with_capacity(20);
        for _ in 0..20 {
            let t0 = Instant::now();
            let _ = e.embed_batch(&batch);
            totals.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        let p50 = percentile(totals.clone(), 0.50);
        let per = p50 / bs as f64;
        println!("  {label:26}  batch={bs:3}  p50={p50:6.2}ms  per-doc={per:.3}ms");
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na < 1e-9 || nb < 1e-9 {
        return 0.0;
    }
    dot / (na * nb)
}

fn main() {
    let texts: Vec<String> = vec![
        "Synapse is a high-performance vector search engine.",
        "Apple M4 Max has 40 GPU cores and 128GB unified memory.",
        "BERT uses bidirectional transformer attention for NLP tasks.",
        "Metal compute shaders accelerate matrix multiplication on Apple Silicon.",
        "BGE-small-en-v1.5 produces 384-dimensional sentence embeddings.",
        "Cosine similarity measures the angle between two vectors.",
        "Rust is a systems programming language with memory safety guarantees.",
        "Candle is a minimalist ML framework for Rust.",
        "Hugging Face hosts pre-trained transformer models.",
        "Mean pooling aggregates token representations into a sentence vector.",
        "L2 normalization projects vectors onto the unit hypersphere.",
        "Retrieval-augmented generation combines search with language models.",
        "Thompson sampling balances exploration and exploitation in bandits.",
        "SimSIMD provides 71× faster similarity computations than scalar code.",
        "SQLite FTS5 supports full-text search with BM25 ranking.",
        "The Matryoshka representation learning method enables variable-length embeddings.",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    println!("=== Candle-Metal BGE-small vs ONNX-CPU bench ===");
    println!("texts={}, iters=100\n", texts.len());

    // --- Candle-Metal ---
    print!("[1/2] Loading Candle-Metal model... ");
    match CandleMetalEmbedder::new("bge-small") {
        Ok(metal_emb) => {
            println!("OK (device: metal)");
            bench_embedder("candle-metal:bge-small", &metal_emb, &texts, 100);

            // --- ONNX-CPU (fastembed) for comparison ---
            #[cfg(any(feature = "embed", feature = "embed-dynamic"))]
            {
                print!("\n[2/2] Loading fastembed ONNX-CPU... ");
                match FastEmbedder::new() {
                    Ok(onnx_emb) => {
                        println!("OK");
                        bench_embedder("fastembed:onnx-cpu", &onnx_emb, &texts, 100);

                        // R@5 parity check: embed same 5 docs, measure cosine.
                        println!("\n--- R@5 parity (Metal vs ONNX cosine similarity) ---");
                        let sample = texts[..5].to_vec();
                        let metal_vecs = metal_emb.embed_batch(&sample).unwrap();
                        let onnx_vecs = onnx_emb.embed_batch(&sample).unwrap();
                        let mut min_cos = f32::MAX;
                        for (i, (mv, ov)) in metal_vecs.iter().zip(&onnx_vecs).enumerate() {
                            let cos = cosine(mv, ov);
                            println!("  doc[{i}] cosine(metal, onnx) = {cos:.4}");
                            if cos < min_cos {
                                min_cos = cos;
                            }
                        }
                        println!("  min cosine = {min_cos:.4}  (≥0.98 = parity OK)");
                        if min_cos >= 0.98 {
                            println!("  PARITY: OK");
                        } else {
                            println!("  PARITY: WARN — cosine < 0.98, check model variant");
                        }
                    }
                    Err(e) => println!("SKIP (fastembed not available: {e})"),
                }
            }
            #[cfg(not(any(feature = "embed", feature = "embed-dynamic")))]
            println!("[2/2] fastembed: SKIP (embed feature not enabled)");
        }
        Err(e) => {
            println!("FAIL: {e}");
            println!("Ensure HF cache has BAAI/bge-small-en-v1.5 or run:");
            println!("  huggingface-cli download BAAI/bge-small-en-v1.5");
            std::process::exit(1);
        }
    }
}
