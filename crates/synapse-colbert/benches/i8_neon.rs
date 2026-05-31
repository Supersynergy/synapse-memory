use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use synapse_kernel::{dot_i8, dot_i8_scalar};

fn bench_dot_i8(c: &mut Criterion) {
    let mut group = c.benchmark_group("dot_i8");
    for dim in [128usize, 256, 512] {
        let a: Vec<i8> = (0..dim).map(|i| (i % 127) as i8).collect();
        let b: Vec<i8> = (0..dim).map(|i| ((127 - i) % 127) as i8).collect();

        group.bench_with_input(BenchmarkId::new("scalar", dim), &dim, |bencher, _| {
            bencher.iter(|| dot_i8_scalar(black_box(&a), black_box(&b)))
        });
        group.bench_with_input(BenchmarkId::new("neon", dim), &dim, |bencher, _| {
            bencher.iter(|| dot_i8(black_box(&a), black_box(&b)))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_dot_i8);
criterion_main!(benches);
