//! Recall bench — search vs search_with_rerank.
//!
//! Run:
//!   cargo bench -p synapse-ann --features ann-usearch --bench recall_cascade
//!
//! Compares recall@10 of plain `search` vs cascade `search_with_rerank(mult=4)`
//! against brute-force ground truth on a 5k × 128d synthetic set.

use criterion::{Criterion, black_box, criterion_group, criterion_main};

#[cfg(feature = "ann-usearch")]
fn vector(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed.wrapping_mul(13) + i as u64).wrapping_mul(7) % 997) as f32 / 997.0 - 0.5)
        .collect()
}

#[cfg(feature = "ann-usearch")]
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    1.0 - dot / (na * nb + 1e-9)
}

#[cfg(feature = "ann-usearch")]
fn ground_truth(vectors: &[Vec<f32>], q: &[f32], k: usize) -> Vec<u64> {
    let mut scored: Vec<(u64, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (i as u64, cosine(q, v)))
        .collect();
    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    scored.into_iter().take(k).map(|(id, _)| id).collect()
}

#[cfg(feature = "ann-usearch")]
fn measure_recall(c: &mut Criterion) {
    use synapse_ann::{AnnIndex, UsearchIndex};
    const DIM: usize = 128;
    const N: usize = 5_000;
    const K: usize = 10;
    const QUERIES: usize = 100;

    let vectors: Vec<Vec<f32>> = (0..N as u64).map(|i| vector(i, DIM)).collect();
    let mut idx = UsearchIndex::new(DIM, N).unwrap();
    for (i, v) in vectors.iter().enumerate() {
        idx.insert(i as u64, v).unwrap();
    }
    let queries: Vec<Vec<f32>> = (0..QUERIES as u64)
        .map(|i| vector(0xdead + i, DIM))
        .collect();

    let truth: Vec<Vec<u64>> = queries
        .iter()
        .map(|q| ground_truth(&vectors, q, K))
        .collect();

    let mut hits_plain = 0usize;
    let mut hits_rerank = 0usize;
    let mut total = 0usize;
    for (q, t) in queries.iter().zip(truth.iter()) {
        let p = idx.search(q, K).unwrap();
        let r = idx.search_with_rerank(q, K, 4).unwrap();
        let t_set: std::collections::HashSet<u64> = t.iter().copied().collect();
        hits_plain += p.iter().filter(|(id, _)| t_set.contains(id)).count();
        hits_rerank += r.iter().filter(|(id, _)| t_set.contains(id)).count();
        total += K;
    }
    let recall_plain = hits_plain as f64 / total as f64;
    let recall_rerank = hits_rerank as f64 / total as f64;
    println!(
        "RECALL@{K}  plain={:.4}  rerank-4x={:.4}  delta={:+.4}",
        recall_plain,
        recall_rerank,
        recall_rerank - recall_plain
    );

    let mut g = c.benchmark_group("recall_cascade");
    g.bench_function("search_plain", |b| {
        b.iter(|| black_box(idx.search(black_box(&queries[0]), K).unwrap()));
    });
    g.bench_function("search_with_rerank_4x", |b| {
        b.iter(|| {
            black_box(
                idx.search_with_rerank(black_box(&queries[0]), K, 4)
                    .unwrap(),
            )
        });
    });
    g.finish();
}

#[cfg(not(feature = "ann-usearch"))]
fn measure_recall(_c: &mut Criterion) {
    eprintln!("skip: enable --features ann-usearch");
}

criterion_group!(benches, measure_recall);
criterion_main!(benches);
