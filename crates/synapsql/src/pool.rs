//! Connection pool + per-connection prepared-statement cache.
//!
//! Patterns stolen from:
//! - Vitess vtgate: statement-plan cache per session
//! - ProxySQL: per-connection statement deduplication
//! - DragonflyDB: shared-nothing, thread-per-core model (approximated via
//!   tokio-task-per-conn + Arc<Store> shared read path)
//!
//! Architecture:
//!   ConnPool owns N logical connection slots.
//!   Each slot holds a PreparedStatementCache (max 1000 entries, LRU).
//!   The backing Store is Arc-shared (read path: zero-copy, write path: serialized).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use parking_lot::Mutex;
use lru::LruCache;
use std::num::NonZeroUsize;

use synapse_libsql::Store;

/// A single prepared statement entry.
#[derive(Clone, Debug)]
pub struct PreparedStmt {
    pub id: u32,
    /// Original SQL text.
    pub sql: String,
    /// Fingerprint (normalized form).
    pub fingerprint: String,
    /// Number of parameters.
    pub num_params: u8,
}

/// Per-connection statement cache (max `cap` entries, LRU eviction).
pub struct StmtCache {
    inner: Mutex<LruCache<u32, PreparedStmt>>,
    /// Monotonic statement-ID counter per connection.
    next_id: AtomicU32,
}

impl StmtCache {
    pub fn new(cap: usize) -> Self {
        let cap = NonZeroUsize::new(cap.max(1)).unwrap();
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            next_id: AtomicU32::new(1),
        }
    }

    /// Register a prepared statement. Returns assigned stmt_id.
    pub fn prepare(&self, sql: &str, fingerprint: &str, num_params: u8) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let stmt = PreparedStmt {
            id,
            sql: sql.to_owned(),
            fingerprint: fingerprint.to_owned(),
            num_params,
        };
        self.inner.lock().put(id, stmt);
        id
    }

    /// Look up a previously prepared statement.
    pub fn get(&self, id: u32) -> Option<PreparedStmt> {
        self.inner.lock().get(&id).cloned()
    }

    /// Remove (close) a statement.
    pub fn close(&self, id: u32) {
        self.inner.lock().pop(&id);
    }

    pub fn len(&self) -> usize { self.inner.lock().len() }
}

/// A logical connection slot.
pub struct Conn {
    pub id: u64,
    pub store: Arc<dyn Store>,
    pub stmts: StmtCache,
}

impl Conn {
    pub fn new(id: u64, store: Arc<dyn Store>, stmt_cap: usize) -> Self {
        Self { id, store, stmts: StmtCache::new(stmt_cap) }
    }
}

/// Connection pool — pre-allocates `size` slots, hands out Arc<Conn>.
/// Lightweight: no actual TCP pooling here (opensrv-mysql owns sockets).
/// This pools the *backend store handles* so we avoid creating a new Arc per query.
pub struct ConnPool {
    store: Arc<dyn Store>,
    next_id: AtomicU64,
    /// QPS counter (reset externally for metrics).
    pub queries: AtomicU64,
    stmt_cap: usize,
}

impl ConnPool {
    pub fn new(store: Arc<dyn Store>, stmt_cap: usize) -> Arc<Self> {
        Arc::new(Self {
            store,
            next_id: AtomicU64::new(1),
            queries: AtomicU64::new(0),
            stmt_cap,
        })
    }

    /// Allocate a new logical connection.
    pub fn acquire(&self) -> Conn {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        Conn::new(id, self.store.clone(), self.stmt_cap)
    }

    /// Record a query execution for QPS tracking.
    pub fn record_query(&self) {
        self.queries.fetch_add(1, Ordering::Relaxed);
    }

    /// Drain and return QPS counter.
    pub fn drain_queries(&self) -> u64 {
        self.queries.swap(0, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synapse_libsql::{Store, QueryResult, LibsqlError};

    struct DummyStore;
    #[async_trait::async_trait]
    impl Store for DummyStore {
        async fn query(&self, _: &str) -> Result<QueryResult, LibsqlError> { Ok(QueryResult::default()) }
        async fn exec(&self, _: &str) -> Result<u64, LibsqlError> { Ok(0) }
    }

    #[test]
    fn stmt_cache_roundtrip() {
        let c = StmtCache::new(10);
        let id = c.prepare("SELECT ?", "select ?", 1);
        let s = c.get(id).unwrap();
        assert_eq!(s.sql, "SELECT ?");
        c.close(id);
        assert!(c.get(id).is_none());
    }

    #[test]
    fn conn_pool_acquire() {
        let pool = ConnPool::new(Arc::new(DummyStore), 100);
        let c1 = pool.acquire();
        let c2 = pool.acquire();
        assert_ne!(c1.id, c2.id);
    }
}
