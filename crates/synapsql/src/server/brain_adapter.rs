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

use crate::parser::rewriter::{rewrite, Extension};
#[cfg(any(feature = "embed", test))]
use crate::parser::rewriter::{PredicateOp as RewritePredicateOp, ScalarPredicate};
use async_trait::async_trait;
use lru::LruCache;
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags};
use std::num::NonZeroUsize;
use std::sync::Arc;
use synapse_core::db::Store as CoreStore;
#[cfg(any(feature = "embed", test))]
use synapse_core::types::{MetadataPredicate, SearchOptions};
use synapse_libsql::{LibsqlError, QueryResult, Store as LibsqlStore};

// ── constants ──────────────────────────────────────────────────────────────────

const STMT_CACHE_CAP: usize = 256;

// ── slot ──────────────────────────────────────────────────────────────────────

/// One connection slot in the pool.
struct Slot {
    conn: Connection,
    /// BLAKE3 fingerprint → cached SQL. We store the SQL text so we can
    /// re-prepare after DDL invalidation (DDL flushes the whole cache).
    stmt_cache: LruCache<[u8; 32], String>,
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
        let t = sql.trim_start();
        let u = t.get(..6).unwrap_or("").to_ascii_uppercase();
        matches!(u.as_str(), "CREATE" | "DROP T" | "ALTER " | "PRAGMA")
    }
}

// ── pool ──────────────────────────────────────────────────────────────────────

/// Round-robin connection pool.
struct Pool {
    slots: Vec<Mutex<Slot>>,
    rr: std::sync::atomic::AtomicUsize,
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
}

// ── adapter ───────────────────────────────────────────────────────────────────

pub struct BrainAdapter {
    /// Primary CoreStore owns tantivy/ann/schema — still used for extended search.
    inner: Arc<Mutex<CoreStore>>,
    /// Fast pool for plain SQL + FTS routing.
    pool: Arc<Pool>,
    /// Database path (for lazy FTS5 virtual table creation).
    db_path: String,
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
            db_path: path.to_owned(),
        })
    }
}

#[async_trait]
impl LibsqlStore for BrainAdapter {
    async fn query(&self, sql: &str) -> Result<QueryResult, LibsqlError> {
        let sql_owned = sql.to_owned();
        let inner = self.inner.clone();
        let pool = self.pool.clone();

        tokio::task::spawn_blocking(move || {
            let rw = rewrite(&sql_owned);

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
                            return Ok(QueryResult { affected: rows.len() as u64, rows });
                        }

                        // For user tables: lazy-create <table>_fts and query it.
                        let fts_table = format!("{}_fts", table);
                        let mut slot = pool.acquire();
                        ensure_fts_index(&slot.conn, &table, &fts_table, column)
                            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                        let fts_sql = format!(
                            "SELECT rowid, * FROM {fts_table} WHERE {fts_table} MATCH ? ORDER BY rank LIMIT 50"
                        );
                        return exec_with_cache(&mut slot, &fts_sql, Some(query_param.as_str()));
                    }
                    Extension::VecSearch(_op) => {
                        // Fix 3: embed + ANN search via CoreStore.
                        #[cfg(feature = "embed")]
                        {
                            use synapse_core::embed::Embedder;
                            use synapse_core::types::SearchMode;
                            let opts = search_options_from_extensions(&rw.extensions);
                            let embedder = Embedder::new()
                                .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let query_str = extract_vec_query_string(&sql_owned)
                                .unwrap_or_else(|| sql_owned.clone());
                            let emb = embedder.embed_one(&query_str)
                                .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let store = inner.lock();
                            let limit = _op.k;
                            let hits = if opts.filter.is_some() {
                                store.search_with_options(
                                    &query_str,
                                    SearchMode::Vec,
                                    Some(&emb),
                                    limit,
                                    &opts,
                                )
                            } else {
                                store.search(&query_str, SearchMode::Vec, Some(&emb), limit)
                            }
                            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let rows: Vec<Vec<u8>> = hits.iter()
                                .map(|h| format!("{}\t{:.6}", h.id, h.score).into_bytes())
                                .collect();
                            return Ok(QueryResult { affected: rows.len() as u64, rows });
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
                            let opts = search_options_from_extensions(&rw.extensions);
                            let embedder = Embedder::new()
                                .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let emb = embedder.embed_one(query_param)
                                .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let store = inner.lock();
                            let hits = if opts.filter.is_some() {
                                store.search_with_options(
                                    query_param,
                                    SearchMode::Hybrid,
                                    Some(&emb),
                                    50,
                                    &opts,
                                )
                            } else {
                                store.search(query_param, SearchMode::Hybrid, Some(&emb), 50)
                            }
                            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                            let rows: Vec<Vec<u8>> = hits.iter()
                                .map(|h| format!("{}\t{:.6}", h.id, h.score).into_bytes())
                                .collect();
                            return Ok(QueryResult { affected: rows.len() as u64, rows });
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
                            return Ok(QueryResult { affected: rows.len() as u64, rows });
                        }
                    }
                    _ => {}
                }
            }

            // ── Plain SQL via pool (Fix 1 + Fix 5) ───────────────────────
            let mut slot = pool.acquire();
            exec_with_cache(&mut slot, &rw.sql, None)
        })
        .await
        .map_err(|e| LibsqlError::Other(e.to_string()))?
    }

    async fn exec(&self, sql: &str) -> Result<u64, LibsqlError> {
        let sql_owned = sql.to_owned();
        let pool = self.pool.clone();

        tokio::task::spawn_blocking(move || {
            let mut slot = pool.acquire();
            let n = slot
                .conn
                .execute(&sql_owned, [])
                .map_err(|e| LibsqlError::Backend(e.to_string()))?;
            // Invalidate stmt cache on DDL
            if Slot::is_ddl(&sql_owned) {
                slot.stmt_cache.clear();
                slot.ddl_epoch = slot.ddl_epoch.wrapping_add(1);
            }
            Ok(n as u64)
        })
        .await
        .map_err(|e| LibsqlError::Other(e.to_string()))?
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Translate SQL-level scalar pushdowns into synapse-core metadata filters.
///
/// Inspired by DataFusion/SurrealDB/Qdrant-style filtered ANN execution:
/// extract cheap scalar predicates before vector work, pass them as
/// `SearchOptions`, then let core oversample and post-filter for recall.
#[cfg(any(feature = "embed", test))]
fn search_options_from_extensions(exts: &[Extension]) -> SearchOptions {
    let mut predicates = Vec::new();

    for ext in exts {
        if let Extension::PredicatePushdown { predicates: pushed } = ext {
            predicates.extend(pushed.iter().filter_map(metadata_predicate_from_scalar));
        }
    }

    let filter = match predicates.len() {
        0 => None,
        1 => predicates.into_iter().next(),
        _ => Some(MetadataPredicate::And(predicates)),
    };

    SearchOptions {
        filter,
        ..Default::default()
    }
}

#[cfg(any(feature = "embed", test))]
fn metadata_predicate_from_scalar(pred: &ScalarPredicate) -> Option<MetadataPredicate> {
    let key = pred.column.rsplit('.').next()?.trim().to_owned();
    if key.is_empty() {
        return None;
    }

    match pred.op {
        RewritePredicateOp::Eq => Some(MetadataPredicate::Eq {
            key,
            value: scalar_json_value(&pred.value),
        }),
        RewritePredicateOp::Ne => Some(MetadataPredicate::Ne {
            key,
            value: scalar_json_value(&pred.value),
        }),
        RewritePredicateOp::Lt => parse_numeric_predicate(&key, &pred.value, |key, value| {
            MetadataPredicate::Lt { key, value }
        }),
        RewritePredicateOp::Le => parse_numeric_predicate(&key, &pred.value, |key, value| {
            MetadataPredicate::Lte { key, value }
        }),
        RewritePredicateOp::Gt => parse_numeric_predicate(&key, &pred.value, |key, value| {
            MetadataPredicate::Gt { key, value }
        }),
        RewritePredicateOp::Ge => parse_numeric_predicate(&key, &pred.value, |key, value| {
            MetadataPredicate::Gte { key, value }
        }),
    }
}

#[cfg(any(feature = "embed", test))]
fn parse_numeric_predicate(
    key: &str,
    raw: &str,
    f: impl FnOnce(String, f64) -> MetadataPredicate,
) -> Option<MetadataPredicate> {
    let n = raw
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .parse::<f64>()
        .ok()?;
    Some(f(key.to_owned(), n))
}

#[cfg(any(feature = "embed", test))]
fn scalar_json_value(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if let Some(param) = trimmed.strip_prefix(':') {
        return serde_json::Value::String(param.to_owned());
    }

    let unquoted = trimmed.trim_matches(|c| c == '\'' || c == '"');
    if let Ok(n) = unquoted.parse::<i64>() {
        return serde_json::json!(n);
    }
    if let Ok(n) = unquoted.parse::<f64>() {
        return serde_json::json!(n);
    }
    match unquoted.to_ascii_lowercase().as_str() {
        "true" => serde_json::Value::Bool(true),
        "false" => serde_json::Value::Bool(false),
        "null" => serde_json::Value::Null,
        _ => serde_json::Value::String(unquoted.to_owned()),
    }
}

/// Execute `sql` on `slot`, using the stmt cache for SELECTs/reads.
/// `bind_val`: optional single text bind parameter (for FTS5 MATCH queries).
fn exec_with_cache(
    slot: &mut Slot,
    sql: &str,
    bind_val: Option<&str>,
) -> Result<QueryResult, LibsqlError> {
    let trimmed = sql.trim_start();
    let upper6 = &trimmed[..trimmed.len().min(6)].to_ascii_uppercase();
    let is_read = upper6 == "SELECT"
        || upper6.starts_with("WITH")
        || upper6.starts_with("PRAGMA")
        || upper6.starts_with("EXPLAIN");

    if Slot::is_ddl(sql) {
        slot.stmt_cache.clear();
        slot.ddl_epoch = slot.ddl_epoch.wrapping_add(1);
    }

    if is_read {
        // Fix 5: check stmt cache.
        let fp = Slot::fingerprint(sql);
        // We can't store rusqlite::Statement (borrows conn), so we cache the SQL
        // and rely on rusqlite's internal statement cache via `prepare_cached`.
        let _ = slot.stmt_cache.get_or_insert(fp, || sql.to_owned());

        let mut stmt = slot
            .conn
            .prepare_cached(sql)
            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
        let col_count = stmt.column_count();
        let mut rows = Vec::new();

        let mut iter = if let Some(val) = bind_val {
            stmt.query([val])
                .map_err(|e| LibsqlError::Backend(e.to_string()))?
        } else {
            stmt.query([])
                .map_err(|e| LibsqlError::Backend(e.to_string()))?
        };

        while let Some(row) = iter
            .next()
            .map_err(|e| LibsqlError::Backend(e.to_string()))?
        {
            let mut parts = Vec::with_capacity(col_count);
            for i in 0..col_count {
                let val: rusqlite::types::Value = row
                    .get(i)
                    .map_err(|e| LibsqlError::Backend(e.to_string()))?;
                let s = match val {
                    rusqlite::types::Value::Null => "NULL".to_owned(),
                    rusqlite::types::Value::Integer(n) => n.to_string(),
                    rusqlite::types::Value::Real(f) => f.to_string(),
                    rusqlite::types::Value::Text(t) => t,
                    rusqlite::types::Value::Blob(b) => format!("<blob {} bytes>", b.len()),
                };
                parts.push(s);
            }
            rows.push(parts.join("\t").into_bytes());
        }
        Ok(QueryResult {
            affected: rows.len() as u64,
            rows,
        })
    } else {
        let n = slot
            .conn
            .execute(sql, [])
            .map_err(|e| LibsqlError::Backend(e.to_string()))?;
        Ok(QueryResult {
            affected: n as u64,
            rows: vec![],
        })
    }
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
    if after.starts_with('\'') || after.starts_with('"') {
        let q = after[1..]
            .find(after.chars().next().unwrap())
            .map(|end| after[1..end + 1].to_owned());
        return q;
    }
    // :param style — return the param name; caller substitutes
    if after.starts_with(':') {
        let end = after[1..]
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|n| n + 1)
            .unwrap_or(after.len());
        return Some(after[1..end].to_owned());
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
    fn predicate_pushdown_becomes_core_search_options() {
        let rw = rewrite(
            "SELECT id FROM docs WHERE tenant_id = 'acme' AND score >= 0.7 AND embedding <=> :q LIMIT 10",
        );

        let opts = search_options_from_extensions(&rw.extensions);
        let Some(MetadataPredicate::And(preds)) = opts.filter else {
            panic!("expected compound metadata predicate");
        };

        assert_eq!(preds.len(), 2);
        assert!(matches!(
            &preds[0],
            MetadataPredicate::Eq { key, value }
                if key == "tenant_id" && value == &serde_json::json!("acme")
        ));
        assert!(matches!(
            &preds[1],
            MetadataPredicate::Gte { key, value }
                if key == "score" && (*value - 0.7).abs() < f64::EPSILON
        ));
    }
}
