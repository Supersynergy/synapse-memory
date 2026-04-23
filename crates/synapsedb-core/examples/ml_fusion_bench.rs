//! ML-based ensemble fusion benchmark.
//!
//! Learns optimal fusion weights for combining multiple search strategies
//! via grid-search maximization of NDCG@10 against f32 ground truth.
//!
//! Run:
//!   cargo run --example ml_fusion_bench -p synapsedb-core --features turbo --release

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::time::Instant;

use synapsedb_core::turbo::binary::BinaryIndex;
use synapsedb_core::turbo::matryoshka::MatryoshkaConfig;
use synapsedb_core::turbo::ndarray_search::NdArraySearch;
use synapsedb_core::turbo::quantize::QuantizedSearch;
use synapsedb_core::turbo::reranker::{EnsembleReranker, Reranker, ScoredResult};
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

/// Compute DCG@k given relevance scores ordered by ranking.
fn dcg_at_k(relevances: &[f64], k: usize) -> f64 {
    let k = k.min(relevances.len());
    relevances[..k]
        .iter()
        .enumerate()
        .map(|(i, &rel)| rel / (2.0f64.ln()) * (i + 2) as f64)
        .sum()
}

/// Compute NDCG@k for a ranked list against ground-truth similarities.
fn ndcg_at_k(
    ranked_ids: &[i64],
    gt_ids: &[i64],
    gt_sims: &[f64],
    k: usize,
) -> f64 {
    let k = k.min(ranked_ids.len()).min(gt_ids.len());
    if k == 0 {
        return 0.0;
    }

    // Build id -> relevance map from ground truth (use similarity as relevance)
    let mut relevance_map = std::collections::HashMap::new();
    for (i, &id) in gt_ids.iter().enumerate() {
        relevance_map.insert(id, gt_sims[i]);
    }

    // Relevances in ranked order
    let ranked_rels: Vec<f64> = ranked_ids
        .iter()
        .map(|id| relevance_map.get(id).copied().unwrap_or(0.0))
        .collect();

    // Ideal DCG: sorted by relevance descending
    let mut ideal_rels: Vec<f64> = gt_sims.to_vec();
    ideal_rels.sort_by(|a, b| b.partial_cmp(a).unwrap());

    let dcg = dcg_at_k(&ranked_rels, k);
    let idcg = dcg_at_k(&ideal_rels, k);
    if idcg < 1e-10 {
        return 1.0;
    }
    dcg / idcg
}

fn main() {
    const K: usize = 10;
    const JSONL_PATH: &str = "/tmp/synapsedb_realbench.jsonl";
    const N_DOCS: usize = 5000;

    eprintln!("SynapseDB ML Ensemble Fusion Benchmark");
    eprintln!("========================================");

    let docs: Vec<_> = load_docs(JSONL_PATH).into_iter().take(N_DOCS).collect();
    eprintln!("Loaded {} documents", docs.len());

    let tmp = build_store(&docs);
    let nd = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let quantized = nd.to_quantized();
    let matryoshka = nd.to_matryoshka(MatryoshkaConfig::default());
    let binary = nd.to_binary(true);

    // Queries: every 50th document
    let queries: Vec<(i64, Vec<f32>)> = (0..docs.len())
        .step_by(50)
        .map(|i| (docs[i].id, docs[i].embedding.clone()))
        .collect();
    eprintln!("Using {} queries (k={})", queries.len(), K);

    // Collect ground truth: f32 search with full similarities
    let mut gt_per_query: Vec<(Vec<i64>, Vec<f64>)> = Vec::with_capacity(queries.len());
    let mut strategy_results: Vec<Vec<Vec<i64>>> = vec![Vec::new(); 4]; // [f32, quant, matry, binary]

    for (_, q) in &queries {
        let f32_res = nd.search(q, K);
        let gt_ids: Vec<i64> = f32_res.iter().map(|(id, _)| *id).collect();
        let gt_sims: Vec<f64> = f32_res.iter().map(|(_, dist)| (1.0 - *dist) as f64).collect();
        gt_per_query.push((gt_ids, gt_sims));

        strategy_results[0].push(nd.search_simd(q, K).iter().map(|(id, _)| *id).collect());
        strategy_results[1].push(quantized.search(q, K).iter().map(|(id, _)| *id).collect());
        strategy_results[2].push(matryoshka.funnel_search(q, K).iter().map(|(id, _)| *id).collect());
        strategy_results[3].push(binary.search_twophase(q, K, 5).iter().map(|(id, _)| *id).collect());
    }

    // ── Individual strategy NDCG ──
    eprintln!("\n# Individual Strategy NDCG@{K}");
    let names = ["f32_simd", "quantized", "matryoshka", "binary"];
    let mut baseline_ndcg = [0.0f64; 4];
    for s in 0..4 {
        let ndcg_sum: f64 = queries
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let (ref gt_ids, ref gt_sims) = gt_per_query[i];
                ndcg_at_k(&strategy_results[s][i], gt_ids, gt_sims, K)
            })
            .sum();
        baseline_ndcg[s] = ndcg_sum / queries.len() as f64;
        eprintln!("  {}: {:.4}", names[s], baseline_ndcg[s]);
    }

    // ── Grid search over ensemble weights ──
    eprintln!("\n# Grid search: 4D weight space (coarse step 0.1)");
    let steps = [0.0f64, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];
    let mut best_weights = vec![0.25f64; 4];
    let mut best_ndcg = 0.0f64;
    let mut evaluated = 0usize;

    for &w0 in &steps {
        for &w1 in &steps {
            for &w2 in &steps {
                for &w3 in &steps {
                    let weights = vec![w0, w1, w2, w3];
                    if weights.iter().sum::<f64>() < 0.1 {
                        continue; // skip all-zero
                    }
                    let ensemble = EnsembleReranker::new(weights);

                    let ndcg_sum: f64 = queries
                        .iter()
                        .enumerate()
                        .map(|(i, _)| {
                            let (ref gt_ids, ref gt_sims) = gt_per_query[i];
                            // Build ScoredResult sources from strategy results
                            let sources: Vec<Vec<ScoredResult>> = (0..4)
                                .map(|s| {
                                    strategy_results[s][i]
                                        .iter()
                                        .enumerate()
                                        .map(|(rank, &id)| ScoredResult {
                                            id,
                                            score: 1.0 / (1.0 + rank as f64),
                                        })
                                        .collect()
                                })
                                .collect();
                            let fused = ensemble.rerank(&sources, K);
                            let fused_ids: Vec<i64> = fused.iter().map(|r| r.id).collect();
                            ndcg_at_k(&fused_ids, gt_ids, gt_sims, K)
                        })
                        .sum();
                    let avg_ndcg = ndcg_sum / queries.len() as f64;
                    evaluated += 1;

                    if avg_ndcg > best_ndcg {
                        best_ndcg = avg_ndcg;
                        best_weights = ensemble.weights.clone();
                    }
                }
            }
        }
    }

    eprintln!("  Evaluated {} weight combinations", evaluated);
    eprintln!(
        "  BEST weights: [{:.2}, {:.2}, {:.2}, {:.2}] → NDCG@{K} = {:.4}",
        best_weights[0], best_weights[1], best_weights[2], best_weights[3], best_ndcg
    );

    // ── Benchmark latency of optimized ensemble ──
    let optimized = EnsembleReranker::new(best_weights.clone());
    let start = Instant::now();
    for i in 0..queries.len() {
        let sources: Vec<Vec<ScoredResult>> = (0..4)
            .map(|s| {
                strategy_results[s][i]
                    .iter()
                    .enumerate()
                    .map(|(rank, &id)| ScoredResult {
                        id,
                        score: 1.0 / (1.0 + rank as f64),
                    })
                    .collect()
            })
            .collect();
        let _ = optimized.rerank(&sources, K);
    }
    let ensemble_latency_us = start.elapsed().as_secs_f64() / queries.len() as f64 * 1e6;

    // ── Summary ──
    eprintln!("\n# Summary");
    eprintln!("  Strategy          | NDCG@{K} | Latency (µs)");
    eprintln!("  ------------------|----------|-------------");
    for s in 0..4 {
        eprintln!("  {:17} | {:.4}   | —", names[s], baseline_ndcg[s]);
    }
    eprintln!(
        "  {:17} | {:.4}   | {:.2}",
        "ensemble (optimal)", best_ndcg, ensemble_latency_us
    );

    let best_single_ndcg = baseline_ndcg.iter().fold(0.0f64, |a, &b| a.max(b));
    let improvement = (best_ndcg - best_single_ndcg) / best_single_ndcg * 100.0;
    eprintln!(
        "\n  Ensemble improvement over best single strategy: +{:.2}% NDCG",
        improvement
    );

    // Print CSV for copy-paste
    println!("\nstrategy,ndcg@{K},latency_us");
    for s in 0..4 {
        println!("{},{:.4},—", names[s], baseline_ndcg[s]);
    }
    println!("ensemble,{:.4},{:.2}", best_ndcg, ensemble_latency_us);
}
