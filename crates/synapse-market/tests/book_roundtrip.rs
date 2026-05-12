use synapse_market::book::{BookEvent, BookStore, Op, Side};
use tempfile::NamedTempFile;

fn make_event(ts: i64, level: u8, side: Side, op: Op, px: f32, qty: f32) -> BookEvent {
    BookEvent {
        ts,
        level,
        side,
        op,
        px,
        qty,
    }
}

fn naive_replay(events: &[BookEvent], ts: i64) -> (f32, f32) {
    let mut bids = [(0.0f32, 0.0f32); 50];
    let mut asks = [(0.0f32, 0.0f32); 50];
    for ev in events.iter().filter(|e| e.ts <= ts) {
        let slot = match ev.side {
            Side::Bid => &mut bids[ev.level as usize % 50],
            Side::Ask => &mut asks[ev.level as usize % 50],
        };
        match ev.op {
            Op::Delete => {
                slot.0 = 0.0;
                slot.1 = 0.0;
            }
            Op::Insert | Op::Update => {
                slot.0 = ev.px;
                slot.1 = ev.qty;
            }
        }
    }
    let best_bid = bids
        .iter()
        .filter(|&&(p, q)| p > 0.0 && q > 0.0)
        .map(|&(p, _)| p)
        .fold(f32::NEG_INFINITY, f32::max);
    let best_ask = asks
        .iter()
        .filter(|&&(p, q)| p > 0.0 && q > 0.0)
        .map(|&(p, _)| p)
        .fold(f32::INFINITY, f32::min);
    (best_bid, best_ask)
}

#[test]
fn roundtrip_10k_events() {
    let tmp = NamedTempFile::new().unwrap();
    let mut events = Vec::with_capacity(10_000);
    let mut base_ts = 1_000_000_000i64;

    for i in 0..10_000usize {
        let side = if i % 2 == 0 { Side::Bid } else { Side::Ask };
        let level = (i % 50) as u8;
        let px = 100.0 + (i % 100) as f32 * 0.01;
        let qty = 1.0 + (i % 10) as f32;
        events.push(make_event(base_ts, level, side, Op::Update, px, qty));
        base_ts += 1_000_000;
    }

    let mut store = BookStore::open(tmp.path()).unwrap();
    store.append(&events).unwrap();

    // Test 10 random timestamps
    let test_ts = [
        events[0].ts,
        events[999].ts,
        events[1000].ts,
        events[2500].ts,
        events[4999].ts,
        events[5000].ts,
        events[7777].ts,
        events[9000].ts,
        events[9500].ts,
        events[9999].ts,
    ];

    for ts in test_ts {
        let (sb, sa) = store.bbo_at(ts).unwrap();
        let (nb, na) = naive_replay(&events, ts);

        // Allow 1 tick tolerance due to f32 delta encoding
        let bid_ok = (sb - nb).abs() < 0.02 || (nb == f32::NEG_INFINITY && sb <= 0.0);
        let ask_ok = (sa - na).abs() < 0.02 || (na == f32::INFINITY && sa >= f32::INFINITY);
        assert!(bid_ok, "ts={ts}: store_bid={sb} naive_bid={nb}");
        assert!(ask_ok, "ts={ts}: store_ask={sa} naive_ask={na}");
    }
}

#[test]
fn events_between_range() {
    let tmp = NamedTempFile::new().unwrap();
    let mut store = BookStore::open(tmp.path()).unwrap();
    let events: Vec<BookEvent> = (0..100)
        .map(|i| {
            make_event(
                1_000_000_000 + i as i64 * 1_000_000,
                (i % 10) as u8,
                if i % 2 == 0 { Side::Bid } else { Side::Ask },
                Op::Update,
                100.0 + i as f32 * 0.1,
                1.0,
            )
        })
        .collect();
    store.append(&events).unwrap();

    let start = events[10].ts;
    let end = events[20].ts;
    let result = store.events_between(start, end).unwrap();
    assert_eq!(result.len(), 11); // [10..=20]
}
