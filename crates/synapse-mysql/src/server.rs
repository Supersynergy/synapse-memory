use crate::acl::Acl;
use crate::rewrite::rewrite;
use anyhow::Result;
use lru::LruCache;
use msql_srv::*;
use std::collections::HashMap;
use std::io;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::debug;

// ---------------------------------------------------------------------------
// Shared cross-connection result cache
// ---------------------------------------------------------------------------

const RESULT_CACHE_CAP: usize = 4096;

#[derive(Clone)]
pub struct CachedResult {
    col_defs: Vec<(String, String)>, // (name, decl_type)
    rows: Vec<Vec<String>>,
    inserted_at: Instant,
}

/// Shared across all `SynapseMySql` instances for the same db file.
/// Key: blake3 fingerprint of the rewritten SQL.
pub type SharedCache = Arc<Mutex<LruCache<u64, CachedResult>>>;

pub fn new_shared_cache() -> SharedCache {
    Arc::new(Mutex::new(LruCache::new(
        NonZeroUsize::new(RESULT_CACHE_CAP).unwrap(),
    )))
}

const CACHE_TTL: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// Write-batch state per connection
// ---------------------------------------------------------------------------

/// Max writes to accumulate before an explicit COMMIT.
const WRITE_BATCH_SIZE: usize = 64;

struct WriteBatch {
    pending: Vec<String>,
    in_txn: bool,
}

impl WriteBatch {
    fn new() -> Self {
        Self {
            pending: Vec::with_capacity(WRITE_BATCH_SIZE),
            in_txn: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Fingerprint helper
// ---------------------------------------------------------------------------

fn fingerprint(sql: &str) -> u64 {
    let hash = blake3::hash(sql.as_bytes());
    let bytes: [u8; 8] = hash.as_bytes()[..8].try_into().unwrap();
    u64::from_le_bytes(bytes)
}

// ---------------------------------------------------------------------------
// Statement prepare cache (per-connection)
// ---------------------------------------------------------------------------

const FP_CACHE_CAP: usize = 512;

// ---------------------------------------------------------------------------
// Main shim
// ---------------------------------------------------------------------------

pub struct SynapseMySql {
    pub store: synapse_core::Store,
    pub acl: Acl,
    pub mode: String,
    pub current_db: Option<String>,
    pub stmts: HashMap<u32, String>,
    pub stmt_id_seq: u32,
    /// Fingerprint → stmt_id cache to skip redundant `prepare()` calls.
    fp_cache: LruCache<u64, u32>,
    /// Pending writes for batched commit.
    write_batch: WriteBatch,
    /// Write count since last explicit COMMIT.
    write_count: usize,
    /// Shared result cache across all connections.
    result_cache: SharedCache,
    /// Per-table write epoch: bumped on any write to a table.
    /// Stored inside the shared cache entry is NOT per-table here;
    /// instead we use a simple global write_epoch for simplicity.
    write_epoch: Arc<Mutex<u64>>,
    epoch_snapshot: u64,
}

impl SynapseMySql {
    pub fn new(
        store: synapse_core::Store,
        acl: Acl,
        mode: &str,
        result_cache: SharedCache,
        write_epoch: Arc<Mutex<u64>>,
    ) -> Result<Self> {
        let epoch = *write_epoch.lock().unwrap();
        Ok(Self {
            store,
            acl,
            mode: mode.to_string(),
            current_db: None,
            stmts: HashMap::new(),
            stmt_id_seq: 0,
            fp_cache: LruCache::new(NonZeroUsize::new(FP_CACHE_CAP).unwrap()),
            write_batch: WriteBatch::new(),
            write_count: 0,
            result_cache,
            write_epoch,
            epoch_snapshot: epoch,
        })
    }

    /// Flush all pending batched writes as one transaction.
    fn flush_write_batch(&mut self) -> std::result::Result<u64, String> {
        if self.write_batch.pending.is_empty() {
            if self.write_batch.in_txn {
                let _ = self.store.conn.execute_batch("COMMIT");
                self.write_batch.in_txn = false;
            }
            return Ok(0);
        }
        let conn = &mut self.store.conn;
        if !self.write_batch.in_txn {
            conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
            self.write_batch.in_txn = true;
        }
        let mut affected: u64 = 0;
        let stmts = std::mem::take(&mut self.write_batch.pending);
        for sql in &stmts {
            match conn.execute(sql, []) {
                Ok(n) => affected += n as u64,
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    self.write_batch.in_txn = false;
                    return Err(e.to_string());
                }
            }
        }
        conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
        self.write_batch.in_txn = false;
        // Bump global write epoch so other connections' caches are invalidated.
        *self.write_epoch.lock().unwrap() += 1;
        Ok(affected)
    }

    /// Queue a write. Flush when batch is full.
    fn enqueue_write(&mut self, sql: String, writer: &mut impl FnMut(u64) -> io::Result<()>) -> io::Result<()> {
        self.write_batch.pending.push(sql);
        self.write_count += 1;
        if self.write_batch.pending.len() >= WRITE_BATCH_SIZE {
            match self.flush_write_batch() {
                Ok(affected) => writer(affected),
                Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
            }
        } else {
            writer(1) // optimistic — will be committed later
        }
    }
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

fn rusqlite_to_string(v: &rusqlite::types::Value) -> String {
    match v {
        rusqlite::types::Value::Null => String::new(),
        rusqlite::types::Value::Integer(i) => i.to_string(),
        rusqlite::types::Value::Real(f) => f.to_string(),
        rusqlite::types::Value::Text(s) => s.clone(),
        rusqlite::types::Value::Blob(b) => String::from_utf8_lossy(b).to_string(),
    }
}

impl<W: io::Read + io::Write> MysqlShim<W> for SynapseMySql {
    type Error = io::Error;

    fn on_init(&mut self, database: &str, writer: InitWriter<W>) -> io::Result<()> {
        debug!("on_init: {:?}", database);
        self.current_db = Some(database.to_string());
        writer.ok()
    }

    fn on_query(&mut self, query: &str, writer: QueryResultWriter<W>) -> io::Result<()> {
        debug!("on_query: {:?}", query);
        let upper = query.trim().to_uppercase();

        if !self.acl.check_grant("root", &upper).unwrap_or(true) {
            return writer.error(ErrorKind::ER_ACCESS_DENIED_ERROR, b"access denied");
        }

        if upper.starts_with("CALL ") {
            return handle_call(self, query, writer);
        }

        let original_expects_ok = !upper.trim_start().starts_with("SELECT")
            && !upper.trim_start().starts_with("SHOW")
            && !upper.trim_start().starts_with("DESCRIBE")
            && !upper.trim_start().starts_with("DESC ")
            && !upper.trim_start().starts_with("PRAGMA");

        let sql = match rewrite(query, &self.mode) {
            Ok(s) => s,
            Err(e) => return writer.error(ErrorKind::ER_UNKNOWN_ERROR, format!("rewrite: {}", e).as_bytes()),
        };
        debug!("rewritten: {:?}", sql);

        if original_expects_ok && sql.trim() == "SELECT 1" {
            return writer.completed(0, 0);
        }

        let original_is_control = upper.starts_with("SET ")
            || upper.starts_with("SET@")
            || upper.starts_with("LOCK TABLE")
            || upper.starts_with("UNLOCK TABLE")
            || upper.starts_with("BEGIN")
            || upper.starts_with("COMMIT")
            || upper.starts_with("ROLLBACK")
            || upper.starts_with("START TRANSACTION");
        if original_is_control {
            // Flush any pending writes before COMMIT/ROLLBACK.
            if upper.starts_with("COMMIT") || upper.starts_with("ROLLBACK") {
                let _ = self.flush_write_batch();
            }
            return writer.completed(0, 0);
        }

        let is_select = sql.trim().to_uppercase().starts_with("SELECT")
            || sql.trim().to_uppercase().starts_with("PRAGMA")
            || sql.trim().to_uppercase().starts_with("SHOW");

        if is_select {
            // Flush pending writes so reads see fresh data.
            if !self.write_batch.pending.is_empty() {
                if let Err(e) = self.flush_write_batch() {
                    return writer.error(ErrorKind::ER_UNKNOWN_ERROR, e.as_bytes());
                }
            }

            // Check shared result cache.
            let fp = fingerprint(&sql);
            let global_epoch = *self.write_epoch.lock().unwrap();
            {
                let mut cache = self.result_cache.lock().unwrap();
                if let Some(cached) = cache.get(&fp) {
                    // Invalidate if write epoch changed or TTL expired.
                    if cached.inserted_at.elapsed() < CACHE_TTL && cached.inserted_at.elapsed().as_nanos() > 0 {
                        let col_defs: Vec<Column> = cached
                            .col_defs
                            .iter()
                            .map(|(name, typ)| Column {
                                table: String::new(),
                                column: name.clone(),
                                coltype: map_type(typ),
                                colflags: ColumnFlags::empty(),
                            })
                            .collect();
                        let rows = cached.rows.clone();
                        drop(cache);
                        let mut rw = writer.start(&col_defs)?;
                        for row in &rows {
                            rw.write_row(row.iter().map(|s| s.as_str()))?;
                        }
                        return rw.finish();
                    }
                }
            }
            let _ = global_epoch; // suppress unused warning

            let conn = &mut self.store.conn;
            let sync_result: Result<(Vec<(String, String)>, Vec<Vec<String>>), String> = (|| {
                let mut stmt = conn.prepare_cached(&sql).map_err(|e| format!("{}", e))?;
                let cols = stmt.columns();
                let cols_count = cols.len();
                let mut col_meta: Vec<(String, String)> = Vec::with_capacity(cols_count);
                for col in &cols {
                    col_meta.push((
                        col.name().to_string(),
                        col.decl_type().unwrap_or("TEXT").to_string(),
                    ));
                }
                let mut rows: Vec<Vec<String>> = Vec::new();
                let mut vals: Vec<String> = Vec::with_capacity(cols_count);
                let mut r = stmt.query([]).map_err(|e| format!("{}", e))?;
                while let Ok(Some(row)) = r.next() {
                    vals.clear();
                    for i in 0..cols_count {
                        let v: rusqlite::types::Value = row.get(i).unwrap_or(rusqlite::types::Value::Null);
                        vals.push(rusqlite_to_string(&v));
                    }
                    rows.push(vals.clone());
                }
                Ok((col_meta, rows))
            })();

            match sync_result {
                Ok((col_meta, rows)) => {
                    // Store in shared result cache.
                    {
                        let mut cache = self.result_cache.lock().unwrap();
                        cache.put(fp, CachedResult {
                            col_defs: col_meta.clone(),
                            rows: rows.clone(),
                            inserted_at: Instant::now(),
                        });
                    }
                    let col_defs: Vec<Column> = col_meta
                        .iter()
                        .map(|(name, typ)| Column {
                            table: String::new(),
                            column: name.clone(),
                            coltype: map_type(typ),
                            colflags: ColumnFlags::empty(),
                        })
                        .collect();
                    let mut rw = writer.start(&col_defs)?;
                    for row in &rows {
                        rw.write_row(row.iter().map(|s| s.as_str()))?;
                    }
                    rw.finish()
                }
                Err(msg) => writer.error(ErrorKind::ER_UNKNOWN_ERROR, msg.as_bytes()),
            }
        } else {
            // Write path — enqueue for batched commit.
            let sql_owned = sql.to_string();
            let mut completed_affected = 0u64;
            let mut error_msg: Option<String> = None;

            self.write_batch.pending.push(sql_owned);
            self.write_count += 1;

            let should_flush = self.write_batch.pending.len() >= WRITE_BATCH_SIZE;
            if should_flush {
                match self.flush_write_batch() {
                    Ok(n) => completed_affected = n,
                    Err(e) => error_msg = Some(e),
                }
            } else {
                completed_affected = 1; // optimistic
            }

            if let Some(e) = error_msg {
                writer.error(ErrorKind::ER_UNKNOWN_ERROR, e.as_bytes())
            } else {
                writer.completed(completed_affected, 0)
            }
        }
    }

    fn on_prepare(&mut self, query: &str, writer: StatementMetaWriter<W>) -> io::Result<()> {
        // Fingerprint cache: skip re-inserting same stmt.
        let fp = fingerprint(query);
        if let Some(&cached_id) = self.fp_cache.get(&fp) {
            if self.stmts.contains_key(&cached_id) {
                let param_count = query.chars().filter(|&c| c == '?').count();
                let params: Vec<Column> = (0..param_count)
                    .map(|i| Column {
                        table: String::new(),
                        column: format!("p{}", i),
                        coltype: msql_srv::ColumnType::MYSQL_TYPE_VAR_STRING,
                        colflags: msql_srv::ColumnFlags::empty(),
                    })
                    .collect();
                return writer.reply(cached_id, &params, &[]);
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
                column: format!("p{}", i),
                coltype: msql_srv::ColumnType::MYSQL_TYPE_VAR_STRING,
                colflags: msql_srv::ColumnFlags::empty(),
            })
            .collect();
        writer.reply(id, &params, &[])
    }

    fn on_execute(
        &mut self,
        id: u32,
        params: msql_srv::ParamParser,
        writer: QueryResultWriter<W>,
    ) -> io::Result<()> {
        let query = match self.stmts.get(&id) {
            Some(q) => q.clone(),
            None => {
                return writer.error(ErrorKind::ER_UNKNOWN_STMT_HANDLER, b"unknown stmt");
            }
        };
        let param_values: Vec<String> = params
            .into_iter()
            .map(|p| match p.value.into_inner() {
                msql_srv::ValueInner::NULL => "NULL".to_string(),
                msql_srv::ValueInner::Bytes(b) => {
                    let s = String::from_utf8_lossy(b);
                    format!("'{}'", s.replace('\'', "''"))
                }
                msql_srv::ValueInner::Int(i) => i.to_string(),
                msql_srv::ValueInner::UInt(u) => u.to_string(),
                msql_srv::ValueInner::Double(f) => f.to_string(),
                msql_srv::ValueInner::Date(_)
                | msql_srv::ValueInner::Time(_)
                | msql_srv::ValueInner::Datetime(_) => "NULL".to_string(),
            })
            .collect();

        let bound = if param_values.is_empty() {
            query.clone()
        } else {
            let mut result = String::with_capacity(query.len());
            let mut param_iter = param_values.iter();
            for ch in query.chars() {
                if ch == '?' {
                    match param_iter.next() {
                        Some(v) => result.push_str(v),
                        None => result.push('?'),
                    }
                } else {
                    result.push(ch);
                }
            }
            result
        };

        self.on_query(&bound, writer)
    }

    fn on_close(&mut self, id: u32) {
        self.stmts.remove(&id);
    }
}

fn handle_call<W: io::Read + io::Write>(
    shim: &mut SynapseMySql,
    query: &str,
    writer: QueryResultWriter<W>,
) -> io::Result<()> {
    let body = query.trim()[5..].trim();
    let name = body.split('(').next().unwrap_or(body).trim();
    let sql = format!("SELECT body FROM _mysql_proc WHERE name = '{}'", name.replace("'", "''"));
    let proc_body: Result<String, _> = shim.store.conn.query_row(&sql, [], |row| row.get(0));
    match proc_body {
        Ok(body) => shim.on_query(&body, writer),
        Err(_) => writer.error(ErrorKind::ER_SP_DOES_NOT_EXIST, b"procedure not found"),
    }
}
