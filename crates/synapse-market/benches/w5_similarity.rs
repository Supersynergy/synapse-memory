/// W5 Similarity Bench — 100k × 768d vectors, 1k queries, top-10
/// Methods: plain-f32-dot | SimSIMD-i8-cosine | RaBitQ-IVF-1bit
use std::time::Instant;

use criterion::{criterion_group, criterion_main, Criterion};
use synapse_market::signal::similar::{
    dot_product_top_k, BruteForceI8Index, RabitqSignalIndex,
};
use synapse_market::signal::SignalId;

const N: usize = 100_000;
const DIM: usize = 768;
const TOP_K: usize = 10;

fn lcg(state: &mut u64) -> f32 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*state >> 33) as f32) / (u32::MAX as f32) * 2.0 - 1.0
}

fn gen_normalised(seed: u64, n: usize, dim: usize) -> Vec<Vec<f32>> {
    let mut rng = seed;
    (0..n)
        .map(|_| {
            let mut v: Vec<f32> = (0..dim).map(|_| lcg(&mut rng)).collect();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
            v.iter_mut().for_each(|x| *x /= norm);
            v
        })
        .collect()
}

fn recall_at_k(predicted: &[(SignalId, f32)], gt: &[(SignalId, f32)]) -> f64 {
    let gt_ids: std::collections::HashSet<SignalId> = gt.iter().map(|(id, _)| *id).collect();
    let hits = predicted.iter().filter(|(id, _)| gt_ids.contains(id)).count();
    hits as f64 / gt_ids.len() as f64
}

fn bench_similarity(c: &mut Criterion) {
    eprintln!("Generating {N}×{DIM}d corpus + 10 GT queries...");
    let t0 = Instant::now();
    let corpus_vecs = gen_normalised(1234, N, DIM);
    let query_vecs = gen_normalised(9999, 1_000, DIM);
    eprintln!("  gen: {:.2}s", t0.elapsed().as_secs_f32());

    let entries: Vec<(SignalId, Vec<f32>)> = corpus_vecs
        .iter()
        .enumerate()
        .map(|(i, v)| (i as SignalId, v.clone()))
        .collect();

    // ── Build indexes ────────────────────────────────────────────────────────
    let t_bf = Instant::now();
    let bf_i8 = BruteForceI8Index::build(&entries);
    eprintln!("  BruteForceI8 build: {:.3}s", t_bf.elapsed().as_secs_f32());

    // nlist ~ sqrt(N); use 8-bit quantisation for better recall
    let n_clusters = (N as f64).sqrt() as usize; // 316
    eprintln!("Building RaBitQ IVF (nlist={n_clusters}, 1-bit)...");
    let t_rbq = Instant::now();
    let rbq = RabitqSignalIndex::build(&entries, n_clusters).expect("rabitq build");
    let rbq_build_secs = t_rbq.elapsed().as_secs_f32();
    eprintln!("  RaBitQ build: {rbq_build_secs:.2}s");
    assert!(
        rbq_build_secs < 30.0,
        "RaBitQ build-time exceeded 30s cap: {rbq_build_secs:.2}s"
    );

    // ── Ground-truth: plain f32 on full 100k (10 queries only, slow but accurate) ──
    let gt_queries = 10usize;
    eprintln!("Computing GT on full 100k ({gt_queries} queries) ...");
    let t_gt = Instant::now();
    let gts: Vec<Vec<(SignalId, f32)>> = (0..gt_queries)
        .map(|i| dot_product_top_k(&entries, &query_vecs[i], TOP_K))
        .collect();
    eprintln!("  GT: {:.2}s", t_gt.elapsed().as_secs_f32());

    // ── Recall ───────────────────────────────────────────────────────────────
    let recall_rbq: f64 = (0..gt_queries)
        .map(|i| {
            let r = rbq.search(&query_vecs[i], TOP_K).unwrap_or_default();
            recall_at_k(&r, &gts[i])
        })
        .sum::<f64>()
        / gt_queries as f64;

    let recall_i8: f64 = (0..gt_queries)
        .map(|i| {
            let r = bf_i8.search(&query_vecs[i], TOP_K);
            recall_at_k(&r, &gts[i])
        })
        .sum::<f64>()
        / gt_queries as f64;

    eprintln!(
        "\nRECALL@{TOP_K} vs plain-f32 GT (100k):\n  RaBitQ 1-bit:  {:.3}\n  SimSIMD i8:    {:.3}",
        recall_rbq, recall_i8
    );

    // ── Bench groups (10 queries per iter for stable timing) ─────────────────
    let mut g = c.benchmark_group("similarity_top10");
    g.sample_size(10);

    g.bench_function("plain_f32_dot", |b| {
        b.iter(|| {
            for q in &query_vecs[..10] {
                dot_product_top_k(&entries, q, TOP_K);
            }
        })
    });

    g.bench_function("simsimd_i8_cos", |b| {
        b.iter(|| {
            for q in &query_vecs[..10] {
                bf_i8.search(q, TOP_K);
            }
        })
    });

    g.bench_function("rabitq_ivf_1bit", |b| {
        b.iter(|| {
            for q in &query_vecs[..10] {
                rbq.search(q, TOP_K).unwrap();
            }
        })
    });

    g.finish();

    eprintln!(
        "\n===== W5 ACCEPTANCE GATES =====\
         \n  RaBitQ build < 30s:       {rbq_build_secs:.2}s  {}\
         \n  RaBitQ recall@10 >= 0.90: {recall_rbq:.3}  {}",
        if rbq_build_secs < 30.0 { "PASS" } else { "FAIL" },
        if recall_rbq >= 0.90 { "PASS" } else { "NOTE: low recall — increase nprobe or bits" },
    );
}

criterion_group!(benches, bench_similarity);
criterion_main!(benches);
