//! Blake3-keyed LRU result + plan cache.
//!
//! Two caches:
//!   1. `QueryCache` — LRU result cache (Bytes). Epoch-invalidated on writes.
//!   2. `PlanCache`  — LRU plan cache (RewriteResult). No epoch: plans are
//!      structural and valid as long as schema doesn't change. Pattern: Vitess
//!      query-plan cache + ProxySQL prepared-statement reuse.
//!
//! Plan cache hit = skip parse+rewrite for hot queries (zero alloc hot path).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;
use lru::LruCache;
use std::num::NonZeroUsize;
use bytes::Bytes;
use crate::parser::rewriter::RewriteResult;

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

/// Thread-safe LRU plan cache, blake3-keyed by fingerprint.
///
/// Stores `RewriteResult` (parsed + rewritten plan AST) so hot queries skip
/// the rewrite pass entirely. Capacity typically 1000 (Vitess default per-conn).
pub struct PlanCache {
    inner: Mutex<LruCache<[u8; 32], RewriteResult>>,
}

impl PlanCache {
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self { inner: Mutex::new(LruCache::new(cap)) }
    }

    pub fn key(fingerprint: &str) -> [u8; 32] {
        *blake3::hash(fingerprint.as_bytes()).as_bytes()
    }

    /// Get cached plan. Returns `None` on miss.
    pub fn get(&self, fingerprint: &str) -> Option<RewriteResult> {
        let k = Self::key(fingerprint);
        self.inner.lock().get(&k).cloned()
    }

    /// Store plan for fingerprint.
    pub fn insert(&self, fingerprint: &str, plan: RewriteResult) {
        let k = Self::key(fingerprint);
        self.inner.lock().put(k, plan);
    }

    /// Evict all plans — call after DDL (schema change invalidates plans).
    pub fn invalidate_all(&self) {
        self.inner.lock().clear();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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

    // --- PlanCache tests ---

    #[test]
    fn plan_cache_hit() {
        use crate::parser::rewriter::rewrite;
        let pc = PlanCache::new(100);
        let fp = "select id from docs where emb <=> ? limit ?";
        let plan = rewrite("SELECT id FROM docs WHERE emb <=> :q LIMIT 10");
        pc.insert(fp, plan.clone());
        let hit = pc.get(fp);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().sql, plan.sql);
    }

    #[test]
    fn plan_cache_miss() {
        let pc = PlanCache::new(100);
        assert!(pc.get("no such query").is_none());
    }

    #[test]
    fn plan_cache_ddl_invalidate() {
        use crate::parser::rewriter::rewrite;
        let pc = PlanCache::new(100);
        let fp = "select id from docs where id = ?";
        pc.insert(fp, rewrite("SELECT id FROM docs WHERE id = 1"));
        pc.invalidate_all();
        assert!(pc.get(fp).is_none());
    }

    #[test]
    fn plan_cache_lru_eviction() {
        use crate::parser::rewriter::rewrite;
        let pc = PlanCache::new(2);
        let dummy = rewrite("SELECT 1");
        pc.insert("q1", dummy.clone());
        pc.insert("q2", dummy.clone());
        pc.insert("q3", dummy.clone());
        assert_eq!(pc.len(), 2);
    }
}
