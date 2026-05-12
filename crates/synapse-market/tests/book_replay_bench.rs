use synapse_market::book::{BookEvent, BookStore, Op, Side};
use std::time::Instant;
use tempfile::NamedTempFile;

#[test]
fn replay_p50_under_50us() {
    let tmp = NamedTempFile::new().unwrap();
    let mut store = BookStore::open(tmp.path()).unwrap();

    // 10k events (smaller than bench, but still covers multiple checkpoints)
    let events: Vec<BookEvent> = (0..10_000usize).map(|i| BookEvent {
        ts: 1_000_000_000 + i as i64 * 1_000,
        level: (i % 50) as u8,
        side: if i % 2 == 0 { Side::Bid } else { Side::Ask },
        op: Op::Update,
        px: 100.0 + (i % 20) as f32 * 0.05,
        qty: 1.0,
    }).collect();
    store.append(&events).unwrap();

    // 1000 random-ish ts queries
    let mut durations = Vec::with_capacity(1000);
    for i in 0..1000usize {
        let ev_idx = (i * 9 + 3) % events.len();
        let ts = events[ev_idx].ts;
        let t0 = Instant::now();
        let _ = store.replay_at(ts).unwrap();
        durations.push(t0.elapsed().as_nanos() as u64);
    }
    durations.sort_unstable();
    let p50_us = durations[499] as f64 / 1000.0;
    let p95_us = durations[949] as f64 / 1000.0;
    let mean_us = durations.iter().sum::<u64>() as f64 / durations.len() as f64 / 1000.0;
    println!("replay p50={p50_us:.1}µs  p95={p95_us:.1}µs  mean={mean_us:.1}µs");

    assert!(p50_us < 50.0, "p50 {p50_us:.1}µs >= 50µs target");
}
