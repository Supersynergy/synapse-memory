/// E2E latency bench for InMemoryF16Index search (brute-force f16 cosine).
/// 10 000 vectors × 384 dims, top-10 query.
/// Reports p50 latency — run with:
///   cargo bench --bench f16_hnsw -p synapse-core -- --bench
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use synapse_core::turbo::inmem_f16_index::InMemoryF16Index;

fn build_index(n: usize, dim: usize) -> InMemoryF16Index {
    let rows: Vec<(i64, Vec<f32>)> = (0..n as i64)
        .map(|i| {
            let v: Vec<f32> = (0..dim)
                .map(|d| {
                    let x = ((i as u64).wrapping_mul(0x9e3779b97f4a7c15)
                        ^ (d as u64 * 0xbf58476d1ce4e5b9)) as f32;
                    x / u64::MAX as f32 - 0.5
                })
                .collect();
            // normalize
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
            (i, v.into_iter().map(|x| x / norm).collect())
        })
        .collect();
    InMemoryF16Index::build(rows)
}

fn query_vec(dim: usize) -> Vec<f32> {
    let v: Vec<f32> = (0..dim)
        .map(|d| {
            let x = (0x1234_5678u64.wrapping_mul(0x9e3779b97f4a7c15)
                ^ (d as u64 * 0xbf58476d1ce4e5b9)) as f32;
            x / u64::MAX as f32 - 0.5
        })
        .collect();
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
    v.into_iter().map(|x| x / norm).collect()
}

fn bench_f16_search(c: &mut Criterion) {
    let mut g = c.benchmark_group("f16_hnsw");

    for &n in &[1_000usize, 10_000, 50_000] {
        let dim = 384;
        let idx = build_index(n, dim);
        let q = query_vec(dim);

        g.bench_with_input(BenchmarkId::new("search_top10", n), &n, |bench, _| {
            bench.iter(|| std::hint::black_box(idx.search(std::hint::black_box(&q), 10)));
        });
    }

    g.finish();
}

criterion_group!(benches, bench_f16_search);
criterion_main!(benches);
