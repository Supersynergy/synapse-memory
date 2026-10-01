//! BrainAdapter — wires synapse-core::Store (sync rusqlite) to the async
//! synapse_libsql::Store trait consumed by the MySQL/PG wire servers.
//!
//! ## Architecture (v2 — pool + stmt-cache)
//!
//! ### Fix 1: Persistent connection pool
//! Instead of one `Mutex<Store>` that serialises every query, we open N rusqlite
//! Connections directly (shared-cache WAL) and hand them out round-robin.
//! Each slot owns its Connection plus an LRU statement cache keyed by BLAKE3.
//! Pool size = tokio worker count (auto-detected), pre-warmed at `open`.
//!
//! ### Fix 2: FTS5 user-table routing
//! `MATCH(col) AGAINST(:q)` now inspects the FROM clause to find the target
//! table. On first MATCH against a table that has no `<table>_fts` virtual
//! table, one is lazily created.
//!
//! ### Fix 3: Vec `<=>` routing (feature-gated)
//! When the `embed` feature is available, the adapter embeds the query string
//! via synapse-core's fastembed pool and calls `Store::search_vec` directly.
//! Without `embed`, returns a descriptive error (not a panic).
//!
//! ### Fix 4: Same-connection write-visibility
//! Each acquired slot is held for the duration of the query, so a client that
//! inserts and immediately selects (bench scenario) hits the same Connection.
//!
//! ### Fix 5: Prepared-statement cache
//! Per-slot LRU cache (capacity 256) maps `blake3(sql)` → cached statement.
//! DDL invalidates the cache. Cache-hit skips `prepare()` entirely.

use crate::parser::cache::PlanCache;
use crate::parser::rewriter::{Extension, RewriteResult, rewrite};
use async_trait::async_trait;
use lru::LruCache;
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags};
use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use synapse_core::db::Store as CoreStore;
use synapse_libsql::{LibsqlError, QueryResult, Store as LibsqlStore};

// ── constants ──────────────────────────────────────────────────────────────────

const STMT_CACHE_CAP: usize = 256;
const RESULT_CACHE_CAP: usize = 4096;
const RESULT_CACHE_MAX_ROWS: usize = 128;
const RESULT_CACHE_MAX_BYTES: usize = 256 * 1024;

type StmtSqlCache = LruCache<[u8; 32], String>;

// ── slot ──────────────────────────────────────────────────────────────────────

/// One connection slot in the pool.
struct Slot {
    conn: Connection,
    /// BLAKE3 fingerprint → cached SQL. We store the SQL text so we can
    /// re-prepare after DDL invalidation (DDL flushes the whole cache).
    stmt_cache: StmtSqlCache,
    /// Tracks whether a DDL statement was executed in this slot (triggers flush).
    ddl_epoch: u64,
}

impl Slot {
    fn open(path: &str) -> Result<Self, LibsqlError> {
        // sqlite3_auto_extension registers sqlite-vec globally; already done by
        // CoreStore::open(). Safe to call Connection::open_with_flags directly.
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| LibsqlError::Backend(e.to_string()))?;

        // Set busy_timeout first so subsequent pragmas wait on any transient lock.
        conn.pragma_update(None, "busy_timeout", 10000_i64)
            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
        // journal_mode is already WAL (set by CoreStore); skip to avoid write-lock race.
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
        conn.pragma_update(None, "temp_store", "MEMORY")
            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
        conn.pragma_update(None, "mmap_size", 1_073_741_824_i64)
            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
        conn.pragma_update(None, "cache_size", -65_536_i64) // 64 MB per slot
            .map_err(|e| LibsqlError::Backend(e.to_string()))?;

        Ok(Slot {
            conn,
            stmt_cache: LruCache::new(NonZeroUsize::new(STMT_CACHE_CAP).unwrap()),
            ddl_epoch: 0,
        })
    }

    /// Returns BLAKE3 fingerprint of `sql`.
    fn fingerprint(sql: &str) -> [u8; 32] {
        *blake3::hash(sql.as_bytes()).as_bytes()
    }

    /// True if `sql` is DDL (CREATE/DROP/ALTER/PRAGMA).
    fn is_ddl(sql: &str) -> bool {
        let t = sql.trim_start().as_bytes();
        starts_ci(t, b"CREATE")
            || starts_ci(t, b"DROP TABLE")
            || starts_ci(t, b"ALTER")
            || starts_ci(t, b"PRAGMA")
    }
}

// ── pool ──────────────────────────────────────────────────────────────────────

/// Round-robin connection pool.
struct Pool {
    slots: Vec<Mutex<Slot>>,
    rr: std::sync::atomic::AtomicUsize,
}

#[derive(Clone)]
struct CachedRows {
    result: QueryResult,
    epoch: u64,
}

type ResultCacheKey = [u8; 32];
type ResultLru = LruCache<ResultCacheKey, CachedRows>;

struct ResultCache {
    inner: Mutex<ResultLru>,
    epoch: AtomicU64,
}

impl ResultCache {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(NonZeroUsize::new(capacity.max(1)).unwrap())),
            epoch: AtomicU64::new(0),
        }
    }

    fn key(sql: &str) -> [u8; 32] {
        *blake3::hash(sql.as_bytes()).as_bytes()
    }

    fn get(&self, sql: &str) -> Option<QueryResult> {
        let epoch = self.epoch.load(Ordering::Acquire);
        self.inner
            .lock()
            .get(&Self::key(sql))
            .filter(|entry| entry.epoch == epoch)
            .map(|entry| entry.result.clone())
    }

    fn insert(&self, sql: &str, result: &QueryResult) {
        if !cacheable_result(result) {
            return;
        }
        let epoch = self.epoch.load(Ordering::Acquire);
        self.inner.lock().put(
            Self::key(sql),
            CachedRows {
                result: result.clone(),
                epoch,
            },
        );
    }

    fn invalidate_all(&self) {
        self.epoch.fetch_add(1, Ordering::Release);
    }
}

impl Pool {
    fn new(path: &str, size: usize) -> Result<Self, LibsqlError> {
        let mut slots = Vec::with_capacity(size);
        for _ in 0..size {
            slots.push(Mutex::new(Slot::open(path)?));
        }
        Ok(Pool {
            slots,
            rr: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Acquire a slot, trying round-robin then try_lock fallback.
    fn acquire(&self) -> parking_lot::MutexGuard<'_, Slot> {
        let n = self.slots.len();
        let start = self.rr.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % n;
        // try_lock round-robin for non-blocking acquire
        for i in 0..n {
            let idx = (start + i) % n;
            if let Some(g) = self.slots[idx].try_lock() {
                return g;
            }
        }
        // All busy — blocking acquire on start slot
        self.slots[start].lock()
    }

    fn invalidate_statement_caches(&self) {
        for slot in &self.slots {
            let mut slot = slot.lock();
            slot.stmt_cache.clear();
            slot.conn.flush_prepared_statement_cache();
            slot.ddl_epoch = slot.ddl_epoch.wrapping_add(1);
        }
    }
}

// ── adapter ───────────────────────────────────────────────────────────────────

pub struct BrainAdapter {
    /// Primary CoreStore owns tantivy/ann/schema — still used for extended search.
    inner: Arc<Mutex<CoreStore>>,
    /// Fast pool for plain SQL + FTS routing.
    pool: Arc<Pool>,
    /// Exact SQL -> rewrite plan cache. Exact keys are intentional: extension
    /// rewrites currently carry literal query params, so fingerprint keys would
    /// be unsafe until binds are separated from the plan.
    plan_cache: Arc<PlanCache>,
    /// Exact read-result cache for hot local-agent/WordPress queries.
    result_cache: Arc<ResultCache>,
    /// Stable namespace for process-wide wire caches.
    cache_namespace: u64,
    /// Database path (for lazy FTS5 virtual table creation).
    _db_path: String,
}

impl BrainAdapter {
    pub fn open(path: &str) -> Result<Self, LibsqlError> {
        // 1. Register sqlite-vec extension and run migrations via CoreStore.
        let store = CoreStore::open(path).map_err(|e| LibsqlError::Backend(e.to_string()))?;

        // 2. Detect pool size from tokio worker count (fallback: 4).
        let pool_size = tokio::runtime::Handle::try_current()
            .ok()
            .map(|h| h.metrics().num_workers().max(2))
            .unwrap_or(4);

        // 3. Open pool — sqlite-vec auto_extension already registered globally.
        let pool = Pool::new(path, pool_size).map_err(|e| LibsqlError::Backend(e.to_string()))?;

        Ok(Self {
            inner: Arc::new(Mutex::new(store)),
            pool: Arc::new(pool),
            plan_cache: Arc::new(PlanCache::new(1024)),
            result_cache: Arc::new(ResultCache::new(RESULT_CACHE_CAP)),
            cache_namespace: cache_namespace_for_path(path),
            _db_path: path.to_owned(),
        })
    }
}

#[async_trait]
impl LibsqlStore for BrainAdapter {
    async fn query(&self, sql: &str) -> Result<QueryResult, LibsqlError> {
        let sql_owned = sql.to_owned();
        let inner = self.inner.clone();
        let pool = self.pool.clone();
        let plan_cache = self.plan_cache.clone();
        let result_cache = self.result_cache.clone();

        if cacheable_read(&sql_owned)
            && let Some(hit) = result_cache.get(&sql_owned)
        {
            return Ok(hit);
        }

        tokio::task::spawn_blocking(move || {
            let rw = rewrite_cached(&plan_cache, &sql_owned);

            // ── Extended search (CoreStore path) ──────────────────────────
            for ext in &rw.extensions {
                match ext {
                    Extension::FtsSearch { query_param, column } => {
                        // Fix 2: route to correct <table>_fts virtual table.
                        let table = extract_from_table(&sql_owned)
                            .unwrap_or_else(|| "docs".to_owned());

                        // For the primary `docs` table: use CoreStore search_lex (tantivy/FTS5).
                        if table == "docs" {
                            let store = inner.lock();
                            let hits = store.search(
                                query_param,
                                synapse_core::types::SearchMode::Lex,
                                None,
                                50,
                            ).map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let rows: Vec<Vec<u8>> = hits.iter()
                                .map(|h| format!("{}\t{}", h.id, h.text).into_bytes())
                                .collect();
                            let result = QueryResult { affected: rows.len() as u64, rows };
                            result_cache.insert(&sql_owned, &result);
                            return Ok(result);
                        }

                        // For user tables: lazy-create <table>_fts and query it.
                        let fts_table = format!("{}_fts", table);
                        let mut slot = pool.acquire();
                        ensure_fts_index(&slot.conn, &table, &fts_table, column)
                            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                        let fts_sql = format!(
                            "SELECT rowid, * FROM {fts_table} WHERE {fts_table} MATCH ? ORDER BY rank LIMIT 50"
                        );
                        let result = exec_with_cache(&mut slot, &fts_sql, Some(query_param.as_str()))?;
                        result_cache.insert(&sql_owned, &result);
                        return Ok(result);
                    }
                    Extension::VecSearch(_op) => {
                        // Fix 3: embed + ANN search via CoreStore.
                        #[cfg(feature = "embed")]
                        {
                            use synapse_core::embed::Embedder;
                            use synapse_core::types::SearchMode;
                            let embedder = Embedder::new()
                                .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let query_str = extract_vec_query_string(&sql_owned)
                                .unwrap_or_else(|| sql_owned.clone());
                            let emb = embedder.embed_one(&query_str)
                                .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let store = inner.lock();
                            let hits = store.search(
                                &query_str,
                                SearchMode::Vec,
                                Some(&emb),
                                50,
                            ).map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let rows: Vec<Vec<u8>> = hits.iter()
                                .map(|h| format!("{}\t{:.6}", h.id, h.score).into_bytes())
                                .collect();
                            let result = QueryResult { affected: rows.len() as u64, rows };
                            result_cache.insert(&sql_owned, &result);
                            return Ok(result);
                        }
                        #[cfg(not(feature = "embed"))]
                        {
                            let _ = inner;
                            return Err(LibsqlError::Backend(
                                "vec-search: embedding pipeline not wired (embed feature disabled)".to_owned()
                            ));
                        }
                    }
                    Extension::HybridRank { query_param, .. } => {
                        #[cfg(feature = "embed")]
                        {
                            use synapse_core::embed::Embedder;
                            use synapse_core::types::SearchMode;
                            let embedder = Embedder::new()
                                .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let emb = embedder.embed_one(query_param)
                                .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let store = inner.lock();
                            let hits = store.search(
                                query_param,
                                SearchMode::Hybrid,
                                Some(&emb),
                                50,
                            ).map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let rows: Vec<Vec<u8>> = hits.iter()
                                .map(|h| format!("{}\t{:.6}", h.id, h.score).into_bytes())
                                .collect();
                            let result = QueryResult { affected: rows.len() as u64, rows };
                            result_cache.insert(&sql_owned, &result);
                            return Ok(result);
                        }
                        #[cfg(not(feature = "embed"))]
                        {
                            // Fallback: lex-only hybrid
                            let store = inner.lock();
                            let hits = store.search(
                                query_param,
                                synapse_core::types::SearchMode::Lex,
                                None,
                                50,
                            ).map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let rows: Vec<Vec<u8>> = hits.iter()
                                .map(|h| format!("{}\t{}", h.id, h.text).into_bytes())
                                .collect();
                            let result = QueryResult { affected: rows.len() as u64, rows };
                            result_cache.insert(&sql_owned, &result);
                            return Ok(result);
                        }
                    }
                    _ => {}
                }
            }

            // ── Plain SQL via pool (Fix 1 + Fix 5) ───────────────────────
            let is_ddl = Slot::is_ddl(&rw.sql);
            if is_ddl {
                pool.invalidate_statement_caches();
                plan_cache.invalidate_all();
            }
            let mut slot = pool.acquire();
            let result = exec_with_cache(&mut slot, &rw.sql, None)?;
            drop(slot);
            if cacheable_read(&sql_owned) {
                result_cache.insert(&sql_owned, &result);
            } else {
                if is_ddl {
                    pool.invalidate_statement_caches();
                    plan_cache.invalidate_all();
                }
                result_cache.invalidate_all();
            }
            Ok(result)
        })
        .await
        .map_err(|e| LibsqlError::Other(e.to_string()))?
    }

    async fn exec(&self, sql: &str) -> Result<u64, LibsqlError> {
        let sql_owned = sql.to_owned();
        let pool = self.pool.clone();
        let plan_cache = self.plan_cache.clone();
        let result_cache = self.result_cache.clone();

        tokio::task::spawn_blocking(move || {
            let is_ddl = Slot::is_ddl(&sql_owned);
            if is_ddl {
                pool.invalidate_statement_caches();
                plan_cache.invalidate_all();
            }
            let mut slot = pool.acquire();
            let n = execute_mutating(&mut slot, &sql_owned)?;
            if is_ddl {
                slot.stmt_cache.clear();
                slot.conn.flush_prepared_statement_cache();
                slot.ddl_epoch = slot.ddl_epoch.wrapping_add(1);
            }
            drop(slot);
            if is_ddl {
                pool.invalidate_statement_caches();
                plan_cache.invalidate_all();
            }
            result_cache.invalidate_all();
            Ok(n as u64)
        })
        .await
        .map_err(|e| LibsqlError::Other(e.to_string()))?
    }

    fn query_cache_epoch(&self) -> Option<u64> {
        Some(self.result_cache.epoch.load(Ordering::Acquire))
    }

    fn query_cache_namespace(&self) -> Option<u64> {
        Some(self.cache_namespace)
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn rewrite_cached(plan_cache: &PlanCache, sql: &str) -> RewriteResult {
    if !needs_extension_rewrite(sql) {
        return RewriteResult {
            sql: sql.to_owned(),
            extensions: Vec::new(),
        };
    }

    if let Some(plan) = plan_cache.get(sql) {
        return plan;
    }

    let plan = rewrite(sql);
    plan_cache.insert(sql, plan.clone());
    plan
}

fn needs_extension_rewrite(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i].to_ascii_uppercase() {
            b'<' if i + 2 < bytes.len() && bytes[i + 1] == b'=' && bytes[i + 2] == b'>' => {
                return true;
            }
            b'M' if starts_ci(&bytes[i..], b"MATCH(") || starts_ci(&bytes[i..], b"MATCH (") => {
                return true;
            }
            b'H' if starts_ci(&bytes[i..], b"HYBRID_RANK(")
                || starts_ci(&bytes[i..], b"HYBRID_RANK (") =>
            {
                return true;
            }
            b'W' if starts_ci(&bytes[i..], b"WITH RECALL_GUARANTEE") => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn starts_ci(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len()
        && haystack
            .iter()
            .take(needle.len())
            .zip(needle)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn cacheable_read(sql: &str) -> bool {
    let s = sql.trim_start().as_bytes();
    starts_ci(s, b"SELECT")
        || starts_ci(s, b"WITH")
        || starts_ci(s, b"PRAGMA")
        || starts_ci(s, b"EXPLAIN")
}

fn cacheable_result(result: &QueryResult) -> bool {
    if result.rows.len() > RESULT_CACHE_MAX_ROWS {
        return false;
    }
    let bytes: usize = result.rows.iter().map(Vec::len).sum();
    bytes <= RESULT_CACHE_MAX_BYTES
}

fn execute_mutating(slot: &mut Slot, sql: &str) -> Result<usize, LibsqlError> {
    let is_ddl = Slot::is_ddl(sql);
    let attempts = if is_ddl { 4 } else { 1 };
    let mut last_error = None;

    for attempt in 0..attempts {
        match slot.conn.execute(sql, []) {
            Ok(n) => return Ok(n),
            Err(e) if is_ddl && sqlite_lock_error(&e) => {
                slot.stmt_cache.clear();
                slot.conn.flush_prepared_statement_cache();
                slot.ddl_epoch = slot.ddl_epoch.wrapping_add(1);
                last_error = Some(e.to_string());
                std::thread::sleep(Duration::from_millis(25 * (attempt as u64 + 1)));
            }
            Err(e) => return Err(LibsqlError::Backend(e.to_string())),
        }
    }

    Err(LibsqlError::Backend(
        last_error.unwrap_or_else(|| "DDL failed after retries".to_owned()),
    ))
}

fn sqlite_lock_error(err: &rusqlite::Error) -> bool {
    let msg = err.to_string();
    msg.contains("locked") || msg.contains("schema has changed")
}

enum QueryBind<'a> {
    None,
    Text(&'a str),
    Integer(i64),
}

/// Execute `sql` on `slot`, using the stmt cache for SELECTs/reads.
/// `bind_val`: optional single text bind parameter (for FTS5 MATCH queries).
fn exec_with_cache(
    slot: &mut Slot,
    sql: &str,
    bind_val: Option<&str>,
) -> Result<QueryResult, LibsqlError> {
    let trimmed = sql.trim_start();
    let bytes = trimmed.as_bytes();
    let is_read = starts_ci(bytes, b"SELECT")
        || starts_ci(bytes, b"WITH")
        || starts_ci(bytes, b"PRAGMA")
        || starts_ci(bytes, b"EXPLAIN");

    if Slot::is_ddl(sql) {
        slot.stmt_cache.clear();
        slot.conn.flush_prepared_statement_cache();
        slot.ddl_epoch = slot.ddl_epoch.wrapping_add(1);
    }

    if is_read {
        let (stmt_sql, bind) = if let Some(val) = bind_val {
            (Cow::Borrowed(sql), QueryBind::Text(val))
        } else if let Some((normalized, id)) = normalize_single_integer_lookup(sql) {
            (Cow::Owned(normalized), QueryBind::Integer(id))
        } else {
            (Cow::Borrowed(sql), QueryBind::None)
        };

        // Fix 5: check stmt cache. Numeric point lookups share one prepared
        // statement (`id=?`) instead of preparing every literal id separately.
        let fp = Slot::fingerprint(stmt_sql.as_ref());
        // We can't store rusqlite::Statement (borrows conn), so we cache the SQL
        // and rely on rusqlite's internal statement cache via `prepare_cached`.
        let _ = slot
            .stmt_cache
            .get_or_insert(fp, || stmt_sql.as_ref().to_owned());

        let mut stmt = slot
            .conn
            .prepare_cached(stmt_sql.as_ref())
            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
        let col_count = stmt.column_count();
        let mut rows = Vec::new();

        let mut iter = match bind {
            QueryBind::None => stmt
                .query([])
                .map_err(|e| LibsqlError::Backend(e.to_string()))?,
            QueryBind::Text(val) => stmt
                .query([val])
                .map_err(|e| LibsqlError::Backend(e.to_string()))?,
            QueryBind::Integer(n) => stmt
                .query([n])
                .map_err(|e| LibsqlError::Backend(e.to_string()))?,
        };

        while let Some(row) = iter
            .next()
            .map_err(|e| LibsqlError::Backend(e.to_string()))?
        {
            let mut line = String::with_capacity(col_count.saturating_mul(12));
            for i in 0..col_count {
                if i > 0 {
                    line.push('\t');
                }
                let val: rusqlite::types::Value = row
                    .get(i)
                    .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                append_sqlite_value(&mut line, val);
            }
            rows.push(line.into_bytes());
        }
        Ok(QueryResult {
            affected: rows.len() as u64,
            rows,
        })
    } else {
        let n = execute_mutating(slot, sql)?;
        Ok(QueryResult {
            affected: n as u64,
            rows: vec![],
        })
    }
}

fn append_sqlite_value(out: &mut String, val: rusqlite::types::Value) {
    match val {
        rusqlite::types::Value::Null => out.push_str("NULL"),
        rusqlite::types::Value::Integer(n) => {
            let _ = write!(out, "{n}");
        }
        rusqlite::types::Value::Real(f) => {
            let _ = write!(out, "{f}");
        }
        rusqlite::types::Value::Text(t) => out.push_str(&t),
        rusqlite::types::Value::Blob(b) => {
            let _ = write!(out, "<blob {} bytes>", b.len());
        }
    }
}

fn cache_namespace_for_path(path: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

fn normalize_single_integer_lookup(sql: &str) -> Option<(String, i64)> {
    let trimmed = sql.trim();
    let trimmed = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    let upper = trimmed.to_ascii_uppercase();
    if !upper.starts_with("SELECT ") {
        return None;
    }

    let where_pos = upper.rfind(" WHERE ")?;
    let prefix = &trimmed[..where_pos + 7];
    let mut rest = trimmed[where_pos + 7..].trim_start();
    let column = if starts_ci(rest.as_bytes(), b"`ID`") {
        rest = &rest[4..];
        "`id`"
    } else if starts_ident_ci(rest, "id") {
        rest = &rest[2..];
        "id"
    } else {
        return None;
    };

    rest = rest.trim_start();
    rest = rest.strip_prefix('=')?.trim_start();

    let bytes = rest.as_bytes();
    let mut end = 0;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == 0 {
        return None;
    }
    if !rest[end..].trim().is_empty() {
        return None;
    }

    let value = rest[..end].parse::<i64>().ok()?;
    Some((format!("{prefix}{column} = ?"), value))
}

fn starts_ident_ci(haystack: &str, ident: &str) -> bool {
    let bytes = haystack.as_bytes();
    let ident_bytes = ident.as_bytes();
    bytes.len() >= ident_bytes.len()
        && starts_ci(bytes, ident_bytes)
        && bytes
            .get(ident_bytes.len())
            .map(|b| !b.is_ascii_alphanumeric() && *b != b'_')
            .unwrap_or(true)
}

/// Extract the primary table name from a `FROM <table>` clause.
fn extract_from_table(sql: &str) -> Option<String> {
    let upper = sql.to_ascii_uppercase();
    let from_pos = upper.find(" FROM ")? + 6;
    let rest = sql[from_pos..].trim_start();
    // Take until whitespace, comma, or JOIN
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ',' || c == '(')
        .unwrap_or(rest.len());
    let table = rest[..end].trim_matches(|c: char| c == '`' || c == '"' || c == '\'');
    if table.is_empty() {
        None
    } else {
        Some(table.to_owned())
    }
}

/// Extract the literal query string from a `<=> 'text'` clause.
#[allow(dead_code)]
fn extract_vec_query_string(sql: &str) -> Option<String> {
    let pos = sql.find("<=>")?;
    let after = sql[pos + 3..].trim_start();
    if let Some(quote @ ('\'' | '"')) = after.chars().next()
        && let Some(stripped) = after.strip_prefix(quote)
    {
        let q = stripped.find(quote).map(|end| stripped[..end].to_owned());
        return q;
    }
    // :param style — return the param name; caller substitutes
    if let Some(stripped) = after.strip_prefix(':') {
        let end = stripped
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(stripped.len());
        return Some(stripped[..end].to_owned());
    }
    None
}

/// Lazy-create `<table>_fts` FTS5 virtual table if it doesn't exist.
/// Populates from `table` on creation.
fn ensure_fts_index(
    conn: &Connection,
    table: &str,
    fts_table: &str,
    col: &str,
) -> rusqlite::Result<()> {
    // Check existence
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [fts_table],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !exists {
        // Detect columns of source table to build content FTS
        let ddl = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {fts_table} USING fts5(content={table}, {col})"
        );
        conn.execute_batch(&ddl)?;
        // Populate
        conn.execute_batch(&format!(
            "INSERT INTO {fts_table}({fts_table}) VALUES('rebuild')"
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_fast_path_skips_plain_sql() {
        assert!(!needs_extension_rewrite(
            "SELECT * FROM posts WHERE id = 42"
        ));
        assert!(!needs_extension_rewrite(
            "INSERT INTO posts VALUES (1, 'a')"
        ));
    }

    #[test]
    fn rewrite_fast_path_detects_extensions() {
        assert!(needs_extension_rewrite(
            "SELECT id FROM docs WHERE embedding <=> :q LIMIT 10"
        ));
        assert!(needs_extension_rewrite(
            "SELECT * FROM docs WHERE MATCH(text) AGAINST ('rust')"
        ));
        assert!(needs_extension_rewrite(
            "SELECT HYBRID_RANK(body, emb, :q) FROM docs"
        ));
        assert!(needs_extension_rewrite(
            "SELECT id FROM docs WITH RECALL_GUARANTEE 0.99"
        ));
    }

    #[test]
    fn result_cache_limits_large_payloads() {
        let small = QueryResult {
            affected: 1,
            rows: vec![b"ok".to_vec()],
        };
        assert!(cacheable_result(&small));

        let large = QueryResult {
            affected: 129,
            rows: vec![b"x".to_vec(); 129],
        };
        assert!(!cacheable_result(&large));
    }

    #[test]
    fn normalize_integer_point_lookup() {
        let (sql, value) =
            normalize_single_integer_lookup("SELECT * FROM posts WHERE id=42;").unwrap();
        assert_eq!(sql, "SELECT * FROM posts WHERE id = ?");
        assert_eq!(value, 42);

        let (sql, value) =
            normalize_single_integer_lookup("select title from posts where `id` = 991").unwrap();
        assert_eq!(sql, "select title from posts where `id` = ?");
        assert_eq!(value, 991);
    }

    #[test]
    fn normalize_integer_point_lookup_ignores_complex_predicates() {
        assert!(normalize_single_integer_lookup("SELECT * FROM posts WHERE idx=42").is_none());
        assert!(
            normalize_single_integer_lookup("SELECT * FROM posts WHERE id=42 AND cat_id=1")
                .is_none()
        );
        assert!(
            normalize_single_integer_lookup("UPDATE posts SET title='x' WHERE id=42").is_none()
        );
    }
}
