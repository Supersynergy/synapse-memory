use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use synapse_ring::RingStore;

fn bench_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("ring");
    group.throughput(Throughput::Elements(100_000));

    group.bench_function("append_100k_st", |b| {
        b.iter(|| {
            let store = RingStore::new();
            for i in 0u64..100_000 {
                store.append(black_box(i.to_le_bytes().to_vec())).unwrap();
            }
            black_box(store.len());
        });
    });

    group.bench_function("append_100k_st_small_payload", |b| {
        let payload = vec![0u8; 32];
        b.iter(|| {
            let store = RingStore::new();
            for _ in 0..100_000u64 {
                store.append(black_box(payload.clone())).unwrap();
            }
            black_box(store.len());
        });
    });

    group.finish();
}

criterion_group!(benches, bench_append);
criterion_main!(benches);
