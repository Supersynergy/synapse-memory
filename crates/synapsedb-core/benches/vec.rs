//! Criterion benchmark — HNSW kNN.
//!
//! Run:
//!   cargo bench -p synapse-core --features vec-hnsw --bench vec

use criterion::{black_box, criterion_group, criterion_main, Criterion};

#[cfg(feature = "vec-hnsw")]
fn bench_knn(c: &mut Criterion) {
    use synapsedb_core::synx::vec_index::HnswIndex;

    fn vector(i: usize, dim: usize) -> Vec<f32> {
        (0..dim)
            .map(|j| ((i * 13 + j * 7) % 997) as f32 / 997.0 - 0.5)
            .collect()
    }

    let vectors: Vec<Vec<f32>> = (0..2000).map(|i| vector(i, 64)).collect();
    let idx = HnswIndex::build(vectors, false).unwrap();
    let q = vector(42, 64);

    c.bench_function("hnsw knn k=10 · 2k × 64d", |b| {
        b.iter(|| {
            let _ = idx.search(black_box(&q), 10);
        })
    });
}

#[cfg(not(feature = "vec-hnsw"))]
fn bench_knn(_c: &mut Criterion) {
    eprintln!("skip: enable --features vec-hnsw");
}

criterion_group!(benches, bench_knn);
criterion_main!(benches);
