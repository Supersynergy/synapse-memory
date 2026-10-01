use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use synapse_mlx_olap::{AggOp, Column, MlxOlapEngine, RecordBatch};

fn make_batch(n: usize, n_groups: usize) -> RecordBatch {
    let mut rng_state: u64 = 42;
    let mut next = || -> f64 {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        (rng_state as f64) / (u64::MAX as f64) * 1000.0
    };

    let values: Vec<f64> = (0..n).map(|_| next()).collect();
    let keys: Vec<String> = (0..n).map(|i| format!("g{}", i % n_groups)).collect();

    RecordBatch::new(
        vec!["grp".to_string(), "val".to_string()],
        vec![Column::Str(keys), Column::Float(values)],
    )
    .unwrap()
}

fn bench_group_by(c: &mut Criterion) {
    let engine = MlxOlapEngine::new().unwrap();
    let mut group = c.benchmark_group("group_by_sum");

    for rows in [1_000_000usize, 10_000_000] {
        let batch = make_batch(rows, 5);
        group.bench_with_input(
            BenchmarkId::new(format!("{:?}", engine.backend), rows),
            &rows,
            |b, _| {
                b.iter(|| {
                    engine
                        .execute_agg(black_box(&batch), AggOp::Sum, "val", Some("grp"))
                        .unwrap()
                });
            },
        );
    }
    group.finish();
}

fn bench_scalar(c: &mut Criterion) {
    let engine = MlxOlapEngine::new().unwrap();
    let batch = make_batch(10_000_000, 1);
    c.bench_function("sum_scalar_10M", |b| {
        b.iter(|| {
            engine
                .execute_agg(black_box(&batch), AggOp::Sum, "val", None)
                .unwrap()
        })
    });
}

criterion_group!(benches, bench_group_by, bench_scalar);
criterion_main!(benches);
