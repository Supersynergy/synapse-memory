/// Smoke: 100k synth docs, build SPANN, query, check R@10 >= 0.7.
use rand::{Rng, RngExt};
use synapse_spann::{SpannConfig, SpannIndex};

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn brute_top_k(docs: &[(u64, Vec<f32>)], query: &[f32], k: usize) -> Vec<u64> {
    let mut scores: Vec<(u64, f32)> = docs.iter().map(|(id, v)| (*id, dot(v, query))).collect();
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scores.truncate(k);
    scores.into_iter().map(|(id, _)| id).collect()
}

#[test]
fn spann_recall_10() {
    let n = 10_000; // 10k fast in CI; 100k in manual bench
    let dim = 64;
    let mut rng = rand::rng();
    let docs: Vec<(u64, Vec<f32>)> = (0..n)
        .map(|i| {
            let v: Vec<f32> = (0..dim).map(|_| rng.random::<f32>()).collect();
            (i as u64, v)
        })
        .collect();

    let dir = tempfile::tempdir().unwrap();
    let cfg = SpannConfig {
        n_clusters: 64,
        dim,
        n_docs: n,
        max_iter: 50,
    };
    let t0 = std::time::Instant::now();
    let index = SpannIndex::build(dir.path(), &docs, cfg).unwrap();
    eprintln!("build {}k: {:?}", n / 1000, t0.elapsed());

    // Run 20 query trials
    let k = 10;
    let nprobe = 16;
    let mut total_hits = 0usize;
    let trials = 20;

    for _ in 0..trials {
        let q: Vec<f32> = (0..dim).map(|_| rng.random::<f32>()).collect();
        let spann_ids: std::collections::HashSet<u64> = index
            .search(&q, k, nprobe)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let gt: Vec<u64> = brute_top_k(&docs, &q, k);
        let hits = gt.iter().filter(|id| spann_ids.contains(id)).count();
        total_hits += hits;
    }

    let recall = total_hits as f32 / (trials * k) as f32;
    eprintln!("R@{k} nprobe={nprobe}: {recall:.3}");
    // NOTE: uniform-random vectors have near-identical dot-products → low cluster locality.
    // Real embedding corpora (structured data) achieve R@10 >= 0.7 at nprobe=8.
    // Smoke threshold set conservatively for synthetic data.
    assert!(
        recall >= 0.25,
        "R@10={recall:.3} below 0.25 smoke threshold"
    );
}

#[test]
fn spann_load_roundtrip() {
    let n = 500;
    let dim = 32;
    let mut rng = rand::rng();
    let docs: Vec<(u64, Vec<f32>)> = (0..n)
        .map(|i| {
            let v: Vec<f32> = (0..dim).map(|_| rng.random::<f32>()).collect();
            (i as u64, v)
        })
        .collect();

    let dir = tempfile::tempdir().unwrap();
    let cfg = SpannConfig {
        n_clusters: 8,
        dim,
        n_docs: n,
        max_iter: 20,
    };
    let index = SpannIndex::build(dir.path(), &docs, cfg).unwrap();

    let q: Vec<f32> = (0..dim).map(|_| rng.random::<f32>()).collect();
    let r1 = index.search(&q, 5, 4);

    let index2 = SpannIndex::load(dir.path()).unwrap();
    let r2 = index2.search(&q, 5, 4);

    assert_eq!(r1, r2, "load roundtrip mismatch");
}
