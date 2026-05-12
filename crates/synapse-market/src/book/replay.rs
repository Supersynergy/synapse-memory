use anyhow::Result;
use std::path::Path;

use super::store::{BookSnapshot, BookStore};

/// Reconstruct the full order-book at exactly `ts`.
pub fn replay_at(path: &Path, ts: i64) -> Result<BookSnapshot> {
    let mut store = BookStore::open(path)?;
    store.replay_at(ts)
}

/// Return (best_bid, best_ask) at `ts`.
pub fn bbo_at(path: &Path, ts: i64) -> Result<(f32, f32)> {
    let mut store = BookStore::open(path)?;
    store.bbo_at(ts)
}
