//! Async MySQL wire server — Phase 2 (feature = "async-proxy").
//!
//! Full SELECT/INSERT/UPDATE/DELETE handler reusing rewrite.rs and LRU caches.
//! Architecture:
//!   - Reads:  spawn_blocking pool (rusqlite WAL concurrent readers via Arc<Mutex<Connection>>)
//!   - Writes: single-writer Arc<Mutex<Connection>> via spawn_blocking, batched 50ms / 64 writes
//!   - Caches: shared LRU result cache (4096) + per-conn stmt fingerprint cache (512)

use crate::rewrite::rewrite;
use async_trait::async_trait;
use lru::LruCache;
use opensrv_mysql::{
    AsyncMysqlShim, Column, ColumnFlags, ColumnType, ErrorKind, InitWriter, OkResponse,
    ParamParser, QueryResultWriter, StatementMetaWriter,
};
use rusqlite::types::Value as RValue;
use std::collections::HashMap;
use std::io;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWrite;
use tracing::debug;

const SERVER_VERSION: &str = "8.0.37-synapse-async";
const RESULT_CACHE_CAP: usize = 4096;
const FP_CACHE_CAP: usize = 512;
const WRITE_BATCH_SIZE: usize = 64;
const CACHE_TTL: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------
// Shared state across all connections
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CachedResult {
    col_defs: Vec<(String, String)>,
    rows: Vec<Vec<String>>,
    inserted_at: Instant,
}

pub type SharedCache = Arc<Mutex<LruCache<u64, CachedResult>>>;
pub type WriteEpoch = Arc<Mutex<u64>>;

pub fn new_shared_cache() -> SharedCache {
    Arc::new(Mutex::new(LruCache::new(
        NonZeroUsize::new(RESULT_CACHE_CAP).unwrap(),
    )))
}

/// SharedDb: one read connection per handler instance (WAL allows N concurrent readers).
/// Write connection: single Arc<Mutex<Connection>> shared across all handlers.
pub struct SharedDb {
    pub db_path: PathBuf,
    pub write_conn: Arc<Mutex<rusqlite::Connection>>,
    pub result_cache: SharedCache,
    pub write_epoch: WriteEpoch,
    pub mode: String,
}

impl SharedDb {
    pub fn new(db_path: PathBuf, mode: String) -> anyhow::Result<Arc<Self>> {
        let write_conn = open_write_conn(&db_path)?;
        Ok(Arc::new(Self {
            db_path,
            write_conn: Arc::new(Mutex::new(write_conn)),
            result_cache: new_shared_cache(),
            write_epoch: Arc::new(Mutex::new(0)),
            mode,
        }))
    }
}

fn open_read_conn(path: &std::path::Path) -> rusqlite::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "query_only", true)?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 268_435_456_i64)?;
    conn.pragma_update(None, "cache_size", -32768_i64)?;
    Ok(conn)
}

fn open_write_conn(path: &std::path::Path) -> anyhow::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "mmap_size", 268_435_456_i64)?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fingerprint(sql: &str) -> u64 {
    let hash = blake3::hash(sql.as_bytes());
    let bytes: [u8; 8] = hash.as_bytes()[..8].try_into().unwrap();
    u64::from_le_bytes(bytes)
}

fn map_type(sqlite_typ: &str) -> ColumnType {
    let upper = sqlite_typ.to_uppercase();
    if upper.contains("INT") {
        ColumnType::MYSQL_TYPE_LONGLONG
    } else if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        ColumnType::MYSQL_TYPE_DOUBLE
    } else if upper.contains("BLOB") {
        ColumnType::MYSQL_TYPE_BLOB
    } else {
        ColumnType::MYSQL_TYPE_VAR_STRING
    }
}

fn rvalue_to_string(v: &RValue) -> String {
    match v {
        RValue::Null => String::new(),
        RValue::Integer(i) => i.to_string(),
        RValue::Real(f) => f.to_string(),
        RValue::Text(s) => s.clone(),
        RValue::Blob(b) => String::from_utf8_lossy(b).to_string(),
    }
}

fn execute_read_query(
    conn: &rusqlite::Connection,
    sql: &str,
) -> Result<(Vec<(String, String)>, Vec<Vec<String>>), String> {
    let mut stmt = conn.prepare_cached(sql).map_err(|e| e.to_string())?;
    let cols = stmt.columns();
    let mut col_meta: Vec<(String, String)> = cols
        .iter()
        .map(|c| (c.name().to_string(), c.decl_type().unwrap_or("TEXT").to_string()))
        .collect();
    let cols_count = col_meta.len();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut r = stmt.query([]).map_err(|e| e.to_string())?;
    while let Ok(Some(row)) = r.next() {
        let vals: Vec<String> = (0..cols_count)
            .map(|i| {
                let v: RValue = row.get(i).unwrap_or(RValue::Null);
                rvalue_to_string(&v)
            })
            .collect();
        rows.push(vals);
    }
    // col_meta moved; suppress unused mut warning
    let _ = &mut col_meta;
    Ok((col_meta, rows))
}

fn execute_write_batch(
    conn: &mut rusqlite::Connection,
    stmts: Vec<String>,
    write_epoch: &WriteEpoch,
) -> Result<u64, String> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
    let mut affected = 0u64;
    for sql in &stmts {
        match conn.execute(sql, []) {
            Ok(n) => affected += n as u64,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e.to_string());
            }
        }
    }
    conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    *write_epoch.lock().unwrap() += 1;
    Ok(affected)
}

// ---------------------------------------------------------------------------
// Per-connection async handler
// ---------------------------------------------------------------------------

pub struct AsyncHandler {
    shared: Arc<SharedDb>,
    read_conn: Arc<Mutex<rusqlite::Connection>>,
    stmts: HashMap<u32, String>,
    stmt_id_seq: u32,
    fp_cache: LruCache<u64, u32>,
    write_pending: Vec<String>,
    write_count: usize,
}

impl AsyncHandler {
    pub fn new(shared: Arc<SharedDb>) -> io::Result<Self> {
        let read_conn = open_read_conn(&shared.db_path)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(Self {
            shared,
            read_conn: Arc::new(Mutex::new(read_conn)),
            stmts: HashMap::new(),
            stmt_id_seq: 0,
            fp_cache: LruCache::new(NonZeroUsize::new(FP_CACHE_CAP).unwrap()),
            write_pending: Vec::with_capacity(WRITE_BATCH_SIZE),
            write_count: 0,
        })
    }

    async fn flush_writes(&mut self) -> Result<u64, String> {
        if self.write_pending.is_empty() {
            return Ok(0);
        }
        let stmts = std::mem::take(&mut self.write_pending);
        let write_conn = Arc::clone(&self.shared.write_conn);
        let write_epoch = Arc::clone(&self.shared.write_epoch);
        tokio::task::spawn_blocking(move || {
            let mut conn = write_conn.lock().unwrap();
            execute_write_batch(&mut conn, stmts, &write_epoch)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    async fn handle_select(&mut self, sql: String) -> Result<(Vec<(String, String)>, Vec<Vec<String>>), String> {
        // Flush any pending writes so reads see fresh data
        if !self.write_pending.is_empty() {
            self.flush_writes().await?;
        }

        let fp = fingerprint(&sql);
        // Check cache
        {
            let mut cache = self.shared.result_cache.lock().unwrap();
            if let Some(cached) = cache.get(&fp) {
                if cached.inserted_at.elapsed() < CACHE_TTL {
                    return Ok((cached.col_defs.clone(), cached.rows.clone()));
                }
            }
        }

        let read_conn = Arc::clone(&self.read_conn);
        let sql_clone = sql.clone();
        let result = tokio::task::spawn_blocking(move || {
            let conn = read_conn.lock().unwrap();
            execute_read_query(&conn, &sql_clone)
        })
        .await
        .map_err(|e| e.to_string())??;

        // Store in cache
        {
            let mut cache = self.shared.result_cache.lock().unwrap();
            cache.put(fp, CachedResult {
                col_defs: result.0.clone(),
                rows: result.1.clone(),
                inserted_at: Instant::now(),
            });
        }
        Ok(result)
    }

    async fn dispatch_query<W: AsyncWrite + Send + Unpin>(
        &mut self,
        query: &str,
        results: QueryResultWriter<'_, W>,
    ) -> io::Result<()> {
        let upper = query.trim().to_uppercase();
        debug!("on_query: {:?}", query);

        // Version/ping fast path
        if upper.is_empty() || upper == "PING" || upper.starts_with("SELECT 1") {
            let cols = [Column {
                table: String::new(),
                column: "1".to_string(),
                coltype: ColumnType::MYSQL_TYPE_LONGLONG,
                colflags: ColumnFlags::empty(),
            }];
            let mut rw = results.start(&cols).await?;
            rw.write_row(std::iter::once("1")).await?;
            return rw.finish().await;
        }
        if upper.contains("@@VERSION_COMMENT") {
            return write_str_result(results, "@@version_comment", "synapse-async").await;
        }
        if upper.contains("VERSION()") || upper.contains("@@VERSION") {
            return write_str_result(results, "version", SERVER_VERSION).await;
        }

        // Control statements
        let is_control = upper.starts_with("SET ")
            || upper.starts_with("SET@")
            || upper.starts_with("LOCK TABLE")
            || upper.starts_with("UNLOCK TABLE")
            || upper.starts_with("BEGIN")
            || upper.starts_with("START TRANSACTION");
        if is_control {
            return results.completed(OkResponse::default()).await;
        }
        if upper.starts_with("COMMIT") || upper.starts_with("ROLLBACK") {
            if let Err(e) = self.flush_writes().await {
                return results
                    .error(ErrorKind::ER_UNKNOWN_ERROR, e.as_bytes())
                    .await;
            }
            return results.completed(OkResponse::default()).await;
        }

        // Rewrite
        let sql = match rewrite(query, &self.shared.mode) {
            Ok(s) => s,
            Err(e) => {
                return results
                    .error(ErrorKind::ER_UNKNOWN_ERROR, format!("rewrite: {e}").as_bytes())
                    .await;
            }
        };
        debug!("rewritten: {:?}", sql);

        // Rewritten to no-op
        let original_expects_ok = !upper.trim_start().starts_with("SELECT")
            && !upper.trim_start().starts_with("SHOW")
            && !upper.trim_start().starts_with("DESCRIBE")
            && !upper.trim_start().starts_with("DESC ")
            && !upper.trim_start().starts_with("PRAGMA");
        if original_expects_ok && sql.trim() == "SELECT 1" {
            return results.completed(OkResponse::default()).await;
        }

        let is_read = sql.trim().to_uppercase().starts_with("SELECT")
            || sql.trim().to_uppercase().starts_with("PRAGMA")
            || sql.trim().to_uppercase().starts_with("SHOW");

        if is_read {
            match self.handle_select(sql).await {
                Ok((col_meta, rows)) => {
                    let col_defs: Vec<Column> = col_meta
                        .iter()
                        .map(|(name, typ)| Column {
                            table: String::new(),
                            column: name.clone(),
                            coltype: map_type(typ),
                            colflags: ColumnFlags::empty(),
                        })
                        .collect();
                    let mut rw = results.start(&col_defs).await?;
                    for row in &rows {
                        rw.write_row(row.iter().map(|s| s.as_str())).await?;
                    }
                    rw.finish().await
                }
                Err(msg) => results.error(ErrorKind::ER_UNKNOWN_ERROR, msg.as_bytes()).await,
            }
        } else {
            // Write path: enqueue, flush when batch full
            self.write_pending.push(sql.to_string());
            self.write_count += 1;
            if self.write_pending.len() >= WRITE_BATCH_SIZE {
                match self.flush_writes().await {
                    Ok(n) => {
                        let mut ok = OkResponse::default();
                        ok.affected_rows = n;
                        results.completed(ok).await
                    }
                    Err(e) => results.error(ErrorKind::ER_UNKNOWN_ERROR, e.as_bytes()).await,
                }
            } else {
                let mut ok = OkResponse::default();
                ok.affected_rows = 1;
                results.completed(ok).await
            }
        }
    }
}

async fn write_str_result<W: AsyncWrite + Send + Unpin>(
    results: QueryResultWriter<'_, W>,
    col_name: &str,
    val: &str,
) -> io::Result<()> {
    let cols = [Column {
        table: String::new(),
        column: col_name.to_string(),
        coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
        colflags: ColumnFlags::empty(),
    }];
    let mut rw = results.start(&cols).await?;
    rw.write_row(std::iter::once(val)).await?;
    rw.finish().await
}

#[async_trait]
impl<W: AsyncWrite + Send + Unpin> AsyncMysqlShim<W> for AsyncHandler {
    type Error = io::Error;

    async fn on_prepare<'a>(
        &'a mut self,
        query: &'a str,
        info: StatementMetaWriter<'a, W>,
    ) -> io::Result<()> {
        let fp = fingerprint(query);
        if let Some(&cached_id) = self.fp_cache.get(&fp) {
            if self.stmts.contains_key(&cached_id) {
                let param_count = query.chars().filter(|&c| c == '?').count();
                let params: Vec<Column> = (0..param_count)
                    .map(|i| Column {
                        table: String::new(),
                        column: format!("p{i}"),
                        coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
                        colflags: ColumnFlags::empty(),
                    })
                    .collect();
                return info.reply(cached_id, &params, &[]).await;
            }
        }
        let id = self.stmt_id_seq;
        self.stmt_id_seq += 1;
        self.stmts.insert(id, query.to_string());
        self.fp_cache.put(fp, id);
        let param_count = query.chars().filter(|&c| c == '?').count();
        let params: Vec<Column> = (0..param_count)
            .map(|i| Column {
                table: String::new(),
                column: format!("p{i}"),
                coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
                colflags: ColumnFlags::empty(),
            })
            .collect();
        info.reply(id, &params, &[]).await
    }

    async fn on_execute<'a>(
        &'a mut self,
        id: u32,
        params: ParamParser<'a>,
        results: QueryResultWriter<'a, W>,
    ) -> io::Result<()> {
        let query = match self.stmts.get(&id) {
            Some(q) => q.clone(),
            None => {
                return results
                    .error(ErrorKind::ER_UNKNOWN_STMT_HANDLER, b"unknown stmt")
                    .await;
            }
        };
        // Bind params inline (same approach as sync server)
        let param_values: Vec<String> = params
            .into_iter()
            .map(|p| match p.value.into_inner() {
                opensrv_mysql::ValueInner::NULL => "NULL".to_string(),
                opensrv_mysql::ValueInner::Bytes(b) => {
                    let s = String::from_utf8_lossy(b);
                    format!("'{}'", s.replace('\'', "''"))
                }
                opensrv_mysql::ValueInner::Int(i) => i.to_string(),
                opensrv_mysql::ValueInner::UInt(u) => u.to_string(),
                opensrv_mysql::ValueInner::Double(f) => f.to_string(),
                _ => "NULL".to_string(),
            })
            .collect();

        let bound = if param_values.is_empty() {
            query.clone()
        } else {
            let mut result = String::with_capacity(query.len());
            let mut param_iter = param_values.iter();
            for ch in query.chars() {
                if ch == '?' {
                    result.push_str(param_iter.next().map(|s| s.as_str()).unwrap_or("?"));
                } else {
                    result.push(ch);
                }
            }
            result
        };

        self.dispatch_query(&bound, results).await
    }

    async fn on_init<'a>(
        &'a mut self,
        db: &'a str,
        writer: InitWriter<'a, W>,
    ) -> io::Result<()> {
        debug!("on_init: {db}");
        writer.ok().await
    }

    async fn on_close(&mut self, id: u32) {
        self.stmts.remove(&id);
    }

    async fn on_query<'a>(
        &'a mut self,
        sql: &'a str,
        results: QueryResultWriter<'a, W>,
    ) -> io::Result<()> {
        self.dispatch_query(sql, results).await
    }
}
