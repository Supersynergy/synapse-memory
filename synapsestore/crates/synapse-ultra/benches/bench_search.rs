use criterion::{criterion_group, criterion_main, Criterion};
use ndarray::Array2;
use synapse_ultra::binary::{build_binary_matrix, pack_signs};
use synapse_ultra::cache::{CacheKey, T0Cache};
use synapse_ultra::index::Hit;
use synapse_ultra::search::{top_k_binary_first, top_k_f32};

fn make_matrix(n: usize, dim: usize) -> (Array2<f32>, Vec<u16>) {
    let data: Vec<f32> = (0..n * dim).map(|i| (i as f32).sin()).collect();
    let mut m = Array2::from_shape_vec((n, dim), data).unwrap();
    synapse_ultra::snapshot::normalize_rows(&mut m);
    let f16: Vec<u16> = m
        .iter()
        .map(|&x| half::f16::from_f32(x).to_bits())
        .collect();
    (m, f16)
}

fn make_query(dim: usize) -> Vec<f32> {
    let mut q: Vec<f32> = (0..dim).map(|i| (i as f32).cos()).collect();
    let norm = q.iter().map(|x| x * x).sum::<f32>().sqrt();
    q.iter_mut().for_each(|x| *x /= norm);
    q
}

fn bench_t0_cache(c: &mut Criterion) {
    let cache = T0Cache::new(32768);
    let hits = vec![Hit { id: 1, score: 0.9 }, Hit { id: 2, score: 0.8 }];
    let key = CacheKey::new("rust async programming", 1, 10);
    cache.put(key, hits);

    c.bench_function("t0_cache_hit", |b| {
        b.iter(|| {
            let key = CacheKey::new("rust async programming", 1, 10);
            cache.get(&key)
        })
    });
}

fn bench_t1_strict_10k(c: &mut Criterion) {
    let (matrix, _) = make_matrix(10_000, 384);
    let q = make_query(384);
    c.bench_function("t1_strict_f32_10k", |b| {
        b.iter(|| top_k_f32(&q, matrix.view(), 10))
    });
}

fn bench_t1_binary_first_10k(c: &mut Criterion) {
    let (matrix, matrix_f16) = make_matrix(10_000, 384);
    let bin = build_binary_matrix(&matrix);
    let q = make_query(384);
    let q_sign = pack_signs(&q);
    c.bench_function("t1_binary_first_10k", |b| {
        b.iter(|| top_k_binary_first(&q, &q_sign, &bin, &matrix_f16, 10_000, 10, 200))
    });
}

fn bench_t1_binary_first_162k(c: &mut Criterion) {
    let n = 162_000usize;
    let (matrix, matrix_f16) = make_matrix(n, 384);
    let bin = build_binary_matrix(&matrix);
    let q = make_query(384);
    let q_sign = pack_signs(&q);
    c.bench_function("t1_binary_first_162k", |b| {
        b.iter(|| top_k_binary_first(&q, &q_sign, &bin, &matrix_f16, n, 10, 200))
    });
}

fn bench_t1_strict_162k(c: &mut Criterion) {
    let n = 162_000usize;
    let (matrix, _) = make_matrix(n, 384);
    let q = make_query(384);
    c.bench_function("t1_strict_f32_162k", |b| {
        b.iter(|| top_k_f32(&q, matrix.view(), 10))
    });
}

criterion_group!(
    benches,
    bench_t0_cache,
    bench_t1_strict_10k,
    bench_t1_binary_first_10k,
    bench_t1_binary_first_162k,
    bench_t1_strict_162k
);
criterion_main!(benches);
