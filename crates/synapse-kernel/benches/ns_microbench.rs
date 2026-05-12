use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use synapse_kernel::kernels::{bin_hamming, f32_l2};

fn bench_l2(c: &mut Criterion) {
    let mut g = c.benchmark_group("f32_l2");
    for &dim in &[128usize, 384, 768, 1536] {
        let a = vec![1.0f32; dim];
        let b = vec![0.5f32; dim];
        g.throughput(Throughput::Elements(dim as u64));
        g.bench_with_input(BenchmarkId::from_parameter(dim), &dim, |b_, _| {
            b_.iter(|| f32_l2::l2_sq(std::hint::black_box(&a), std::hint::black_box(&b)));
        });
    }
    g.finish();
}

fn bench_hamming(c: &mut Criterion) {
    let mut g = c.benchmark_group("bin_hamming");
    for &words in &[2usize, 4, 8, 16, 32] {
        // 128-2048 bit
        let a = vec![0xAAAA_AAAA_AAAA_AAAAu64; words];
        let b = vec![0x5555_5555_5555_5555u64; words];
        g.throughput(Throughput::Bytes((words * 8) as u64));
        g.bench_with_input(BenchmarkId::from_parameter(words * 64), &words, |b_, _| {
            b_.iter(|| {
                bin_hamming::hamming_u64(std::hint::black_box(&a), std::hint::black_box(&b))
            });
        });
    }
    g.finish();
}

criterion_group!(benches, bench_l2, bench_hamming);
criterion_main!(benches);
