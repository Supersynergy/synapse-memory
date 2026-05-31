use criterion::*;
use rand::{Rng, SeedableRng, rngs::StdRng};
use synapse_ann::{AnnIndex, UsearchIndex};

fn norm(v: &mut [f32]) {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 1e-10 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

fn synth(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let mut v: Vec<f32> = (0..dim).map(|_| rng.r#gen::<f32>() - 0.5).collect();
            norm(&mut v);
            v
        })
        .collect()
}

fn ground_truth(corpus: &[Vec<f32>], queries: &[Vec<f32>], k: usize) -> Vec<Vec<usize>> {
    queries
        .iter()
        .map(|q| {
            let mut scores: Vec<(usize, f32)> = corpus
                .iter()
                .enumerate()
                .map(|(i, v)| (i, q.iter().zip(v).map(|(a, b)| a * b).sum::<f32>()))
                .collect();
            scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            scores[..k].iter().map(|(i, _)| *i).collect()
        })
        .collect()
}

fn bench_config(
    c: &mut Criterion,
    name: &str,
    m: usize,
    ef_c: usize,
    ef_s: usize,
    corpus: &[Vec<f32>],
    queries: &[Vec<f32>],
    gt: &[Vec<usize>],
) {
    const K: usize = 10;
    let q_count = queries.len();
    let dim = corpus[0].len();
    let n = corpus.len();

    let mut idx = UsearchIndex::new_tuned(dim, n, m, ef_c, ef_s).unwrap();
    for (i, v) in corpus.iter().enumerate() {
        idx.insert(i as u64, v).unwrap();
    }

    let mut recall_total = 0;
    for (qi, q) in queries.iter().enumerate() {
        let hits = idx.search(q, K).unwrap();
        let hit_ids: std::collections::HashSet<u64> = hits.iter().map(|(id, _)| *id).collect();
        let gt_set: std::collections::HashSet<u64> = gt[qi].iter().map(|i| *i as u64).collect();
        recall_total += hit_ids.intersection(&gt_set).count();
    }
    let recall = recall_total as f32 / (q_count * K) as f32;
    println!("=== {name} M={m} ef_c={ef_c} ef_s={ef_s} -> recall@10={recall:.4} ===",);

    let q0 = &queries[0];
    c.bench_function(&format!("hnsw_{name}_M{m}_ec{ef_c}_es{ef_s}"), |b| {
        b.iter(|| black_box(idx.search(q0, K).unwrap()))
    });
}

fn benches(c: &mut Criterion) {
    const N: usize = 50_000;
    const D: usize = 384;
    const Q: usize = 50;
    const K: usize = 10;

    let corpus = synth(N, D, 42);
    let queries = synth(Q, D, 999);
    let gt = ground_truth(&corpus, &queries, K);

    bench_config(c, "A", 32, 200, 64, &corpus, &queries, &gt);
    bench_config(c, "B", 48, 400, 128, &corpus, &queries, &gt);
    bench_config(c, "C", 64, 200, 64, &corpus, &queries, &gt);
    bench_config(c, "default", 16, 256, 256, &corpus, &queries, &gt);
}

criterion_group!(g, benches);
criterion_main!(g);
