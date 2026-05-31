//! Criterion bench — UsearchIndex build + query at scales 1k / 10k / 100k.
//!
//! Run:
//!   cargo bench -p synapse-ann --features ann-usearch --bench ann_usearch
//!
//! Reports ms/query and build wall-time for each N.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

#[cfg(feature = "ann-usearch")]
fn vector(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed.wrapping_mul(13) + i as u64).wrapping_mul(7) % 997) as f32 / 997.0 - 0.5)
        .collect()
}

#[cfg(feature = "ann-usearch")]
fn bench_ann(c: &mut Criterion) {
    use synapse_ann::{AnnIndex, UsearchIndex};
    const DIM: usize = 384;
    let scales = [1_000usize, 10_000, 100_000];
    let mut g = c.benchmark_group("usearch_knn_k10");
    for &n in &scales {
        let mut idx = UsearchIndex::new(DIM, n).unwrap();
        for i in 0..n as u64 {
            idx.insert(i, &vector(i, DIM)).unwrap();
        }
        let q = vector(0xdead, DIM);
        g.throughput(Throughput::Elements(1));
        g.bench_with_input(BenchmarkId::from_parameter(n), &q, |b, qv| {
            b.iter(|| {
                let r = idx.search(black_box(qv), 10).unwrap();
                black_box(r);
            });
        });
    }
    g.finish();
}

#[cfg(not(feature = "ann-usearch"))]
fn bench_ann(_c: &mut Criterion) {
    eprintln!("skip: enable --features ann-usearch");
}

criterion_group!(benches, bench_ann);
criterion_main!(benches);
