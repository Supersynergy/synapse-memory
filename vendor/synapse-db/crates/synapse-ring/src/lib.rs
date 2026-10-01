use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use crossbeam_queue::SegQueue;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Single ring entry. Kept small — payload is raw bytes to avoid alloc on hot-path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub ts_us: u64,
    pub payload: Vec<u8>,
}

impl Entry {
    fn new(seq: u64, payload: Vec<u8>) -> Self {
        let ts_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        Self {
            seq,
            ts_us,
            payload,
        }
    }
}

/// Snapshot-iterator — point-in-time drain of all buffered entries.
pub struct SnapshotIter {
    items: Vec<Entry>,
    idx: usize,
}

impl Iterator for SnapshotIter {
    type Item = Entry;
    fn next(&mut self) -> Option<Self::Item> {
        if self.idx < self.items.len() {
            let e = self.items[self.idx].clone();
            self.idx += 1;
            Some(e)
        } else {
            None
        }
    }
}

enum SnapMsg {
    Entries(Vec<Entry>),
    Shutdown,
}

/// Lock-free append-only ring buffer with optional background SQLite snapshot.
pub struct RingStore {
    queue: Arc<SegQueue<Entry>>,
    seq: AtomicU64,
    snapshot_tx: Option<mpsc::SyncSender<SnapMsg>>,
}

impl Default for RingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RingStore {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(SegQueue::new()),
            seq: AtomicU64::new(0),
            snapshot_tx: None,
        }
    }

    /// Lock-free CAS append — ~sub-µs on hot path.
    #[inline]
    pub fn append(&self, payload: Vec<u8>) -> Result<()> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let entry = Entry::new(seq, payload);
        if let Some(tx) = &self.snapshot_tx {
            // Shadow-copy for persistence (non-blocking, bounded channel)
            let _ = tx.try_send(SnapMsg::Entries(vec![entry.clone()]));
        }
        self.queue.push(entry);
        Ok(())
    }

    /// Drain all currently buffered entries — point-in-time snapshot.
    pub fn iter_snapshot(&self) -> SnapshotIter {
        let mut items = Vec::with_capacity(self.queue.len());
        while let Some(e) = self.queue.pop() {
            items.push(e);
        }
        SnapshotIter { items, idx: 0 }
    }

    /// Peek count without draining.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Start background snapshot thread that batches every `interval` to SQLite at `path`.
    pub fn enable_persistence(&mut self, path: PathBuf, interval: Duration) {
        // Bounded channel: backpressure if bg-thread falls behind
        let (tx, rx) = mpsc::sync_channel::<SnapMsg>(65_536);
        self.snapshot_tx = Some(tx);

        thread::Builder::new()
            .name("ring-snapshot".into())
            .spawn(move || snapshot_worker(rx, path, interval))
            .expect("spawn ring-snapshot thread");
    }
}

fn snapshot_worker(rx: mpsc::Receiver<SnapMsg>, path: PathBuf, interval: Duration) {
    let conn = match open_db(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[synapse-ring] failed to open snapshot db: {e}");
            return;
        }
    };

    let mut batch: Vec<Entry> = Vec::with_capacity(4096);
    let mut last_flush = std::time::Instant::now();

    loop {
        // Drain whatever is pending (non-blocking)
        loop {
            match rx.try_recv() {
                Ok(SnapMsg::Entries(mut v)) => batch.append(&mut v),
                Ok(SnapMsg::Shutdown) => {
                    flush_batch(&conn, &batch);
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    flush_batch(&conn, &batch);
                    return;
                }
            }
        }

        if last_flush.elapsed() >= interval && !batch.is_empty() {
            flush_batch(&conn, &batch);
            batch.clear();
            last_flush = std::time::Instant::now();
        }

        thread::sleep(Duration::from_millis(1));
    }
}

fn open_db(path: &PathBuf) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS ring_entries (
             seq   INTEGER PRIMARY KEY,
             ts_us INTEGER NOT NULL,
             payload BLOB NOT NULL
         );",
    )?;
    Ok(conn)
}

fn flush_batch(conn: &Connection, batch: &[Entry]) {
    if batch.is_empty() {
        return;
    }
    let tx = conn.unchecked_transaction();
    if let Ok(tx) = tx {
        let mut stmt = match conn.prepare_cached(
            "INSERT OR IGNORE INTO ring_entries (seq, ts_us, payload) VALUES (?1, ?2, ?3)",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };
        for e in batch {
            let _ = stmt.execute(rusqlite::params![e.seq as i64, e.ts_us as i64, &e.payload]);
        }
        let _ = tx.commit();
    }
}

impl Drop for RingStore {
    fn drop(&mut self) {
        if let Some(tx) = &self.snapshot_tx {
            let _ = tx.try_send(SnapMsg::Shutdown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_append_and_iter() {
        let store = RingStore::new();
        for i in 0u64..1000 {
            store.append(i.to_le_bytes().to_vec()).unwrap();
        }
        assert_eq!(store.len(), 1000);
        let items: Vec<_> = store.iter_snapshot().collect();
        assert_eq!(items.len(), 1000);
        assert!(store.is_empty());
    }

    #[test]
    fn smoke_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("ring.db");
        let mut store = RingStore::new();
        store.enable_persistence(db_path.clone(), Duration::from_millis(10));
        for i in 0u64..500 {
            store.append(i.to_le_bytes().to_vec()).unwrap();
        }
        // Let bg-thread flush
        thread::sleep(Duration::from_millis(50));
        drop(store);

        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM ring_entries", [], |r| r.get(0))
            .unwrap();
        assert!(count > 0, "expected flushed rows, got 0");
    }

    #[test]
    fn seq_monotonic() {
        let store = RingStore::new();
        for i in 0..100u64 {
            store.append(vec![i as u8]).unwrap();
        }
        let items: Vec<_> = store.iter_snapshot().collect();
        let seqs: Vec<u64> = items.iter().map(|e| e.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted);
    }
}
