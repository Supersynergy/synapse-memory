use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use half::f16;
use synapse_kernel::kernels::f16_dot::{dot_f16, dot_f16_scalar};

fn bench_f16_dot(c: &mut Criterion) {
    let mut g = c.benchmark_group("f16_dot");
    for &dim in &[128usize, 256, 512] {
        let a: Vec<f16> = (0..dim).map(|i| f16::from_f32(i as f32 * 0.01)).collect();
        let b: Vec<f16> = (0..dim)
            .map(|i| f16::from_f32((dim - i) as f32 * 0.01))
            .collect();

        g.throughput(Throughput::Elements(dim as u64));

        g.bench_with_input(BenchmarkId::new("scalar", dim), &dim, |bench, _| {
            bench.iter(|| dot_f16_scalar(std::hint::black_box(&a), std::hint::black_box(&b)));
        });

        g.bench_with_input(BenchmarkId::new("neon", dim), &dim, |bench, _| {
            bench.iter(|| dot_f16(std::hint::black_box(&a), std::hint::black_box(&b)));
        });
    }
    g.finish();
}

criterion_group!(benches, bench_f16_dot);
criterion_main!(benches);
