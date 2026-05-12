//! Blake3-keyed LRU result cache.
//!
//! Pattern: Vitess query-plan cache + DuckDB hot-table-cache.
//! Write-epoch invalidation: any write bumps the epoch, reads with
//! stale epoch are cache-miss (conformal invalidation).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;
use lru::LruCache;
use std::num::NonZeroUsize;
use bytes::Bytes;

/// Cached result entry.
#[derive(Clone, Debug)]
pub struct CachedResult {
    /// Serialized MySQL wire-frame rows.
    pub payload: Bytes,
    /// Write epoch at cache-fill time.
    pub epoch: u64,
    /// Number of columns.
    pub ncols: usize,
}

/// Thread-safe LRU query result cache, blake3-keyed by fingerprint.
pub struct QueryCache {
    inner: Mutex<LruCache<[u8; 32], CachedResult>>,
    /// Monotonic write epoch — bump on any INSERT/UPDATE/DELETE/DDL.
    epoch: Arc<AtomicU64>,
}

impl QueryCache {
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Blake3 hash of a query fingerprint string.
    pub fn key(fingerprint: &str) -> [u8; 32] {
        *blake3::hash(fingerprint.as_bytes()).as_bytes()
    }

    /// Get a cached result. Returns `None` if missing or epoch-stale.
    pub fn get(&self, fingerprint: &str) -> Option<CachedResult> {
        let k = Self::key(fingerprint);
        let current_epoch = self.epoch.load(Ordering::Acquire);
        let mut inner = self.inner.lock();
        inner.get(&k).filter(|r| r.epoch == current_epoch).cloned()
    }

    /// Insert a result. Tagged with current epoch.
    pub fn insert(&self, fingerprint: &str, payload: Bytes, ncols: usize) {
        let k = Self::key(fingerprint);
        let epoch = self.epoch.load(Ordering::Acquire);
        let mut inner = self.inner.lock();
        inner.put(k, CachedResult { payload, epoch, ncols });
    }

    /// Bump write epoch — invalidates all cached reads.
    pub fn invalidate_all(&self) {
        self.epoch.fetch_add(1, Ordering::Release);
    }

    /// Peek at current epoch (for metrics).
    pub fn current_epoch(&self) -> u64 {
        self.epoch.load(Ordering::Relaxed)
    }

    /// Current cache length.
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit() {
        let c = QueryCache::new(100);
        let fp = "select * from t where id = ?";
        c.insert(fp, Bytes::from_static(b"data"), 2);
        assert!(c.get(fp).is_some());
    }

    #[test]
    fn invalidate_on_write() {
        let c = QueryCache::new(100);
        let fp = "select * from t where id = ?";
        c.insert(fp, Bytes::from_static(b"data"), 2);
        c.invalidate_all();
        assert!(c.get(fp).is_none());
    }

    #[test]
    fn lru_eviction() {
        let c = QueryCache::new(2);
        c.insert("q1", Bytes::from_static(b"a"), 1);
        c.insert("q2", Bytes::from_static(b"b"), 1);
        c.insert("q3", Bytes::from_static(b"c"), 1);
        assert_eq!(c.len(), 2);
    }
}
