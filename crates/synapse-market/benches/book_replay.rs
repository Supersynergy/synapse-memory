use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use synapse_market::book::{BookEvent, BookStore, Op, Side};
use tempfile::NamedTempFile;

fn gen_events(n: usize) -> Vec<BookEvent> {
    (0..n).map(|i| BookEvent {
        ts: 1_000_000_000 + i as i64 * 1_000,
        level: (i % 50) as u8,
        side: if i % 2 == 0 { Side::Bid } else { Side::Ask },
        op: Op::Update,
        px: 100.0 + (i % 100) as f32 * 0.01,
        qty: 1.0 + (i % 10) as f32,
    }).collect()
}

fn bench_book_replay(c: &mut Criterion) {
    let n = 1_000_000;
    let events = gen_events(n);

    let tmp = NamedTempFile::new().unwrap();
    let mut store = BookStore::open(tmp.path()).unwrap();

    // Append in chunks to avoid huge stack
    for chunk in events.chunks(50_000) {
        store.append(chunk).unwrap();
    }

    // Precompute query timestamps
    let queries: Vec<i64> = (0..100).map(|i| {
        let ev_idx = (i * 9973 + 7) % n;
        events[ev_idx].ts
    }).collect();

    let mut group = c.benchmark_group("book_replay");
    group.bench_function(BenchmarkId::new("synapse_x_checkpoint", n), |b| {
        b.iter(|| {
            for &ts in &queries {
                criterion::black_box(store.replay_at(ts).unwrap());
            }
        });
    });

    // Naive: linear scan from t=0 for each query
    group.bench_function(BenchmarkId::new("naive_linear_scan", n), |b| {
        b.iter(|| {
            for &ts in &queries {
                let mut bids = [(0.0f32, 0.0f32); 50];
                let mut asks = [(0.0f32, 0.0f32); 50];
                for ev in events.iter().take_while(|e| e.ts <= ts) {
                    let slot = match ev.side {
                        Side::Bid => &mut bids[ev.level as usize],
                        Side::Ask => &mut asks[ev.level as usize],
                    };
                    slot.0 = ev.px; slot.1 = ev.qty;
                }
                let _ = criterion::black_box((bids[0], asks[0]));
            }
        });
    });

    group.finish();
}

criterion_group!(benches, bench_book_replay);
criterion_main!(benches);
