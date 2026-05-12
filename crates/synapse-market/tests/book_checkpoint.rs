use synapse_market::book::{BookEvent, BookStore, Op, Side};
use tempfile::NamedTempFile;

#[test]
fn checkpoint_every_1000_events() {
    let tmp = NamedTempFile::new().unwrap();
    let mut store = BookStore::open(tmp.path()).unwrap();

    // Initial checkpoint written on first append
    assert_eq!(store.n_checkpoints(), 0);

    let batch: Vec<BookEvent> = (0..1000).map(|i| BookEvent {
        ts: 1_000_000_000 + i as i64 * 1000,
        level: (i % 50) as u8,
        side: Side::Bid,
        op: Op::Update,
        px: 100.0,
        qty: 1.0,
    }).collect();

    store.append(&batch).unwrap();
    // Should have: initial checkpoint + 1 checkpoint after 1000 events = 2
    assert_eq!(store.n_checkpoints(), 2, "expected 2 checkpoints after 1000 events");

    // Append another 1000
    let batch2: Vec<BookEvent> = (0..1000).map(|i| BookEvent {
        ts: 2_000_000_000 + i as i64 * 1000,
        level: (i % 50) as u8,
        side: Side::Ask,
        op: Op::Update,
        px: 101.0,
        qty: 2.0,
    }).collect();
    store.append(&batch2).unwrap();
    assert_eq!(store.n_checkpoints(), 3, "expected 3 checkpoints after 2000 events");
    assert_eq!(store.n_events(), 2000);
}

#[test]
fn checkpoint_partial_batch() {
    let tmp = NamedTempFile::new().unwrap();
    let mut store = BookStore::open(tmp.path()).unwrap();

    // 500 events — should NOT generate extra checkpoint
    let batch: Vec<BookEvent> = (0..500).map(|i| BookEvent {
        ts: i as i64 * 1000,
        level: 0,
        side: Side::Bid,
        op: Op::Update,
        px: 50.0,
        qty: 1.0,
    }).collect();
    store.append(&batch).unwrap();
    assert_eq!(store.n_checkpoints(), 1, "only initial checkpoint for <1000 events");
    assert_eq!(store.n_events(), 500);
}
