//! Hyperparameter optimization sweep for turbo search strategies.
//!
//! Sweeps:
//! - Matryoshka: coarse_dim ∈ {48, 64, 96, 128}, funnel_factor ∈ {2, 4, 6, 8}
//! - Binary: overselect ∈ {5, 10, 15, 20, 30, 50}
//!
//! Run:
//!   cargo run --example hyperopt_bench -p synapsedb-core --features turbo --release

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::time::Instant;

use synapsedb_core::turbo::binary::BinaryIndex;
use synapsedb_core::turbo::matryoshka::MatryoshkaConfig;
use synapsedb_core::turbo::ndarray_search::NdArraySearch;
use synapsedb_core::turbo::quantize::QuantizedSearch;
use synapsedb_core::{PutRequest, Store};

#[derive(Debug, Clone, serde::Deserialize)]
struct Doc {
    id: i64,
    #[allow(dead_code)]
    title: String,
    text: String,
    embedding: Vec<f32>,
}

fn load_docs(path: &str) -> Vec<Doc> {
    let file = File::open(path).expect("open jsonl");
    BufReader::new(file)
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).expect("parse json"))
        .collect()
}

fn build_store(docs: &[Doc]) -> tempfile::NamedTempFile {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let mut store = Store::open(tmp.path()).unwrap();
    for doc in docs {
        store
            .put(&PutRequest {
                title: Some(doc.title.clone()),
                text: doc.text.clone(),
                embedding: Some(doc.embedding.clone()),
                ..Default::default()
            })
            .unwrap();
    }
    tmp
}

fn recall_at_k(ground_truth: &[i64], results: &[(i64, f32)], k: usize) -> f32 {
    let gt_set: std::collections::HashSet<_> = ground_truth[..k.min(ground_truth.len())].iter().collect();
    let overlap = results.iter().take(k).filter(|(id, _)| gt_set.contains(id)).count();
    overlap as f32 / k as f32
}

fn main() {
    const K: usize = 10;
    const JSONL_PATH: &str = "/tmp/synapsedb_realbench.jsonl";
    const N_DOCS: usize = 5000;

    eprintln!("SynapseDB Hyperparameter Optimization");
    eprintln!("======================================");

    let docs: Vec<_> = load_docs(JSONL_PATH).into_iter().take(N_DOCS).collect();
    eprintln!("Loaded {} documents", docs.len());

    let tmp = build_store(&docs);
    let nd = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let quantized = nd.to_quantized();

    // Queries: every 50th document
    let queries: Vec<(i64, Vec<f32>)> = (0..docs.len())
        .step_by(50)
        .map(|i| (docs[i].id, docs[i].embedding.clone()))
        .collect();
    eprintln!("Using {} queries (k={})", queries.len(), K);

    // Ground truth
    let gt: Vec<Vec<i64>> = queries
        .iter()
        .map(|(_, q)| nd.search(q, K).iter().map(|(id, _)| *id).collect())
        .collect();

    println!("\n# Matryoshka sweep: coarse_dim × funnel_factor");
    println!("coarse_dim,funnel_factor,latency_us,recall@10");

    for &coarse_dim in &[48_usize, 64, 96, 128] {
        for &funnel_factor in &[2_usize, 4, 6, 8] {
            let config = MatryoshkaConfig {
                coarse_dim,
                funnel_factor,
            };
            let matryoshka = nd.to_matryoshka(config);

            let start = Instant::now();
            for (_, q) in &queries {
                let _ = matryoshka.funnel_search(q, K);
            }
            let latency = start.elapsed().as_secs_f64() / queries.len() as f64 * 1e6;

            let recall: f32 = queries
                .iter()
                .enumerate()
                .map(|(i, (_, q))| recall_at_k(&gt[i], &matryoshka.funnel_search(q, K), K))
                .sum::<f32>()
                / queries.len() as f32;

            println!("{},{},{:.2},{:.4}", coarse_dim, funnel_factor, latency, recall);
        }
    }

    println!("\n# Binary sweep: overselect factor");
    println!("overselect,latency_us,recall@10");

    let binary = nd.to_binary(true);
    for &overselect in &[5, 10, 15, 20, 30, 50] {
        let start = Instant::now();
        for (_, q) in &queries {
            let _ = binary.search_twophase(q, K, overselect);
        }
        let latency = start.elapsed().as_secs_f64() / queries.len() as f64 * 1e6;

        let recall: f32 = queries
            .iter()
            .enumerate()
            .map(|(i, (_, q))| recall_at_k(&gt[i], &binary.search_twophase(q, K, overselect), K))
            .sum::<f32>()
            / queries.len() as f32;

        println!("{},{:.2},{:.4}", overselect, latency, recall);
    }

    println!("\n# Quantized baseline");
    println!("strategy,latency_us,recall@10");
    let start = Instant::now();
    for (_, q) in &queries {
        let _ = quantized.search(q, K);
    }
    let latency = start.elapsed().as_secs_f64() / queries.len() as f64 * 1e6;
    let recall: f32 = queries
        .iter()
        .enumerate()
        .map(|(i, (_, q))| recall_at_k(&gt[i], &quantized.search(q, K), K))
        .sum::<f32>()
        / queries.len() as f32;
    println!("quantized,{:.2},{:.4}", latency, recall);

    println!("\n# SIMD f32 baseline");
    let start = Instant::now();
    for (_, q) in &queries {
        let _ = nd.search_simd(q, K);
    }
    let latency = start.elapsed().as_secs_f64() / queries.len() as f64 * 1e6;
    println!("simd_f32,{:.2},1.0000", latency);

    eprintln!("\nHyperparameter sweep complete.");
}
