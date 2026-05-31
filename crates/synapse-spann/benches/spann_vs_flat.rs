use criterion::{Criterion, black_box, criterion_group, criterion_main};
use rand::{Rng, RngExt};
use std::time::Instant;
use synapse_spann::{SpannConfig, SpannIndex};

fn gen_docs(n: usize, dim: usize) -> Vec<(u64, Vec<f32>)> {
    let mut rng = rand::rng();
    (0..n)
        .map(|i| {
            let v: Vec<f32> = (0..dim).map(|_| rng.random::<f32>()).collect();
            (i as u64, v)
        })
        .collect()
}

fn bench_spann(c: &mut Criterion) {
    let n = 10_000;
    let dim = 128;
    let docs = gen_docs(n, dim);
    let dir = tempfile::tempdir().unwrap();

    let cfg = SpannConfig {
        n_clusters: 64,
        dim,
        n_docs: n,
        max_iter: 50,
    };
    let t0 = Instant::now();
    let index = SpannIndex::build(dir.path(), &docs, cfg).unwrap();
    eprintln!("SPANN build {}k docs: {:?}", n / 1000, t0.elapsed());

    let mut rng = rand::rng();
    let query: Vec<f32> = (0..dim).map(|_| rng.random::<f32>()).collect();

    c.bench_function("spann_search_nprobe8", |b| {
        b.iter(|| black_box(index.search(&query, 10, 8)));
    });

    // Flat baseline: brute-force dot-product
    c.bench_function("flat_brute_search", |b| {
        b.iter(|| {
            let mut scores: Vec<(u64, f32)> = docs
                .iter()
                .map(|(id, v)| {
                    let s: f32 = v.iter().zip(&query).map(|(a, b)| a * b).sum();
                    (*id, s)
                })
                .collect();
            scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            scores.truncate(10);
            black_box(scores)
        });
    });
}

criterion_group!(benches, bench_spann);
criterion_main!(benches);
