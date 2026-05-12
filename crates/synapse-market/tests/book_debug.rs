use synapse_market::book::{BookEvent, BookStore, Op, Side};
use tempfile::NamedTempFile;

#[test]
fn debug_10k() {
    let tmp = NamedTempFile::new().unwrap();
    let mut events = Vec::with_capacity(10_000);
    let mut base_ts = 1_000_000_000i64;
    for i in 0..10_000usize {
        let side = if i % 2 == 0 { Side::Bid } else { Side::Ask };
        let level = (i % 50) as u8;
        let px = 100.0 + (i % 100) as f32 * 0.01;
        let qty = 1.0 + (i % 10) as f32;
        events.push(BookEvent { ts: base_ts, level, side, op: Op::Update, px, qty });
        base_ts += 1_000_000;
    }
    let mut store = BookStore::open(tmp.path()).unwrap();
    store.append(&events).unwrap();
    println!("n_events={} n_checkpoints={}", store.n_events(), store.n_checkpoints());

    // Test at ts of last event
    let ts = events[9999].ts;
    let snap = store.replay_at(ts).unwrap();
    let bb = snap.best_bid();
    let ba = snap.best_ask();
    println!("ts={ts} best_bid={bb} best_ask={ba}");
    println!("bids[0]={:?}", snap.bids[0]);
    println!("bids[48]={:?}", snap.bids[48]);

    // Also test at ts of event 5000
    let ts5 = events[5000].ts;
    let snap5 = store.replay_at(ts5).unwrap();
    println!("ts5={ts5} best_bid5={} best_ask5={}", snap5.best_bid(), snap5.best_ask());
}
