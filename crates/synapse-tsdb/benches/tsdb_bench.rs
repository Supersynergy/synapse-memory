use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::collections::HashMap;
use synapse_tsdb::fallback::{AggOp, Row, TsdbStore};
use tempfile::TempDir;

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("tsdb");
    group.throughput(Throughput::Elements(1_000_000));
    group.bench_function("insert_1M", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            let mut store = TsdbStore::open(dir.path()).unwrap();
            let base = 1_700_000_000_000i64;
            for i in 0..1_000_000i64 {
                store
                    .append_row(Row {
                        ts: base + i * 100,
                        metric: "cpu".to_string(),
                        labels: HashMap::new(),
                        value: (i % 100) as f64,
                    })
                    .unwrap();
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_insert);
criterion_main!(benches);
