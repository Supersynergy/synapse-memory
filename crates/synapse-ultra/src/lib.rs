//! Synapse Ultra — lean extension layer for synapse-memory.
//!
//! Adds four capabilities on top of the existing SQLite brain.db:
//!   1. `synapse_events` — beads-style event log (what happened when, by whom)
//!   2. `graph_nodes` / `graph_edges` — graph-v2 as SQLite-CTE (replaces broken Datalog)
//!   3. `why()` / `graph_expand()` — recursive CTE views for decision-chain traversal
//!   4. Observe CLI — inspect / why / replay / cost / events / graph / lake
//!
//! Design goals (ADR 0004-0008):
//!   - No modification to existing synapse-core schema (additive only)
//!   - Idempotent migration (`synapse-ultra init` is safe to run repeatedly)
//!   - Single-file storage (same brain.db, or own ultra.db)
//!   - Production-ready: WAL, indexes, BLAKE3 dedup, zstd compression for large content
//!
//! # Example
//! ```no_run
//! use synapse_ultra::Ultra;
//! let ultra = Ultra::open("/path/to/brain.db")?;
//! ultra.migrate()?;
//! ultra.with_conn(|c| synapse_ultra::events::ingest_event_json(
//!     c,
//!     r#"{"agent":"claude","kind":"decision","uri":"file:foo.rs","content":"refactored"}"#,
//! ))?;
//! let chain = ultra.with_conn(|c| synapse_ultra::graph::why(c, "file:foo.rs", 5))?;
//! # Ok::<(), synapse_ultra::UltraError>(())
//! ```

pub mod error;
pub mod events;
pub mod graph;
pub mod lake;
pub mod observe;
pub mod ops;
pub mod schema;
pub mod tags;

pub use error::{UltraError, UltraResult};
pub use events::{Event, EventFilter, EventKind};
pub use graph::{GraphEdge, GraphNode, WhyStep};
pub use lake::LakeState;
pub use observe::{
    AgentSummary, AgentTraceRow, BrainStats, CostRow, DailySummary, DecisionRow, ReplayEntry,
    SessionRow, SessionTimelineRow,
};
pub use ops::{HealthCheck, HealthReport, health_check};
pub use tags::{DocTag, Tag, TagExport, TagRule, TagStats};

use parking_lot::Mutex;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static MEM_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Apply perf pragmas to a fresh connection.
fn apply_pragmas(conn: &Connection) -> UltraResult<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)?;
    conn.pragma_update(None, "cache_size", -65536_i64)?; // 64MB page cache
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 268435456_i64)?; // 256MB mmap
    conn.pragma_update(None, "wal_autocheckpoint", 1000_i64)?;
    Ok(())
}

/// A minimal SQLite connection pool — `Mutex<Vec<Connection>>` with
/// lazy creation. WAL mode supports concurrent readers; writers serialize
/// via SQLite's file lock. Avoids the `r2d2_sqlite` version conflict with
/// the workspace's pinned `rusqlite 0.39`.
struct ConnPool {
    path: PathBuf,
    conns: Mutex<Vec<Connection>>,
    max_size: usize,
}

impl ConnPool {
    fn new(path: PathBuf, max_size: usize) -> UltraResult<Self> {
        let first = Connection::open(&path)?;
        apply_pragmas(&first)?;
        Ok(Self {
            path,
            conns: Mutex::new(vec![first]),
            max_size,
        })
    }

    fn memory(max_size: usize) -> UltraResult<Self> {
        // Unique shared-cache in-memory DB per instance so concurrent tests
        // don't interfere. URI: `file:ultra_mem_{id}?mode=memory&cache=shared`.
        let id = MEM_COUNTER.fetch_add(1, Ordering::Relaxed);
        let uri = format!("file:ultra_mem_{id}?mode=memory&cache=shared");
        let conn = Connection::open(&uri)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self {
            path: PathBuf::from(uri),
            conns: Mutex::new(vec![conn]),
            max_size: 1.max(max_size),
        })
    }

    fn get(&self) -> UltraResult<Connection> {
        let mut guard = self.conns.lock();
        if let Some(c) = guard.pop() {
            return Ok(c);
        }
        // Pool exhausted — create a new connection to the same DB.
        // Works for file (WAL shared state) and shared-cache memory URIs.
        let conn = Connection::open(&self.path)?;
        apply_pragmas(&conn)?;
        Ok(conn)
    }

    fn put(&self, conn: Connection) {
        let mut guard = self.conns.lock();
        if guard.len() < self.max_size {
            guard.push(conn);
        }
        // else: drop the extra connection
    }

    fn size(&self) -> usize {
        self.conns.lock().len()
    }
}

/// The Synapse Ultra handle. Wraps a small SQLite connection pool.
///
/// Thread-safe via the pool — WAL mode supports concurrent readers, and the
/// pool size bounds concurrent access. Default pool: 8 connections.
pub struct Ultra {
    pool: ConnPool,
    path: PathBuf,
}

impl Ultra {
    /// Open or create a Synapse Ultra database at `path`.
    ///
    /// If `path` points to an existing synapse-memory `brain.db`, the new
    /// tables are added alongside `docs`/`docs_fts`/`docs_vec` without
    /// modifying them. If `path` does not exist, a fresh database is created.
    pub fn open<P: AsRef<Path>>(path: P) -> UltraResult<Self> {
        let path = path.as_ref().to_path_buf();
        let pool = ConnPool::new(path.clone(), 8)?;
        Ok(Self { pool, path })
    }

    /// Open an in-memory database (for tests). Single connection.
    pub fn open_memory() -> UltraResult<Self> {
        let pool = ConnPool::memory(1)?;
        Ok(Self {
            pool,
            path: PathBuf::from(":memory:"),
        })
    }

    /// Path of the underlying database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run the idempotent schema migration. Safe to call repeatedly.
    ///
    /// Adds: `synapse_events`, `graph_nodes`, `graph_edges`, `decisions`,
    /// `sessions`, `token_cost`, and the `why_chain` / `graph_expand_views` SQL views.
    pub fn migrate(&self) -> UltraResult<()> {
        let conn = self.pool.get()?;
        let r = schema::migrate(&conn);
        self.pool.put(conn);
        r
    }

    /// Access a pooled connection (for advanced users / tests).
    ///
    /// Checks out a connection from the pool for the duration of the closure,
    /// then returns it. Concurrent callers get their own connection — WAL
    /// mode allows parallel readers; writers still serialize via SQLite's
    /// file lock.
    pub fn with_conn<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Connection) -> R,
    {
        let conn = self.pool.get().expect("pool get failed");
        let r = f(&conn);
        self.pool.put(conn);
        r
    }

    /// Current pool size (number of idle connections).
    pub fn pool_size(&self) -> usize {
        self.pool.size()
    }
}
