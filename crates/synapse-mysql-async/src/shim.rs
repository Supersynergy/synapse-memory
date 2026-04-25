//! AsyncMysqlShim impl backed by Synapse Store via spawn_blocking.
//!
//! Strategy:
//! - tokio AsyncMysqlShim handles wire IO async
//! - Per-connection dedicated rusqlite Connection (read-only)
//! - All sync rusqlite calls go through `tokio::task::spawn_blocking`
//! - Reuse `synapse_mysql::rewrite::rewrite()` and ACL logic
//! - Shared LRU result cache across connections (parking_lot::Mutex)
//!
//! Phase 1 MVP: text protocol only. Prepare/execute return graceful errors.
//! Phase 1.5: full prepare statement support.

use anyhow::Result;
use async_trait::async_trait;
use lru::LruCache;
use opensrv_mysql::{
    AsyncMysqlShim, Column, ColumnFlags, ColumnType, ErrorKind, InitWriter, ParamParser,
    QueryResultWriter, StatementMetaWriter, ValueInner,
};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::collections::HashMap;
use std::io;
use std::num::NonZeroUsize;

// ── wp_options autoload cache ─────────────────────────────────────────────────
const AUTOLOAD_PATTERNS: &[&str] = &[
    "select option_value from wp_options where autoload",
    "select option_name, option_value from wp_options where autoload",
    "select * from wp_options where autoload",
];

fn is_autoload_query(sql: &str) -> bool {
    let lo = sql.to_ascii_lowercase();
    AUTOLOAD_PATTERNS.iter().any(|p| lo.contains(p))
}

fn is_wp_options_write(sql: &str) -> bool {
    let lo = sql.to_ascii_lowercase();
    let is_write = lo.starts_with("insert")
        || lo.starts_with("update")
        || lo.starts_with("delete")
        || lo.starts_with("replace");
    is_write && lo.contains("wp_options")
}
fn is_dml_write(sql: &str) -> bool {
    let upper = sql.trim_start().to_uppercase();
    upper.starts_with("INSERT")
        || upper.starts_with("UPDATE")
        || upper.starts_with("DELETE")
        || upper.starts_with("REPLACE")
}

// ── wp_postmeta covering-index optimization ───────────────────────────────────
/// Returns true for WP's canonical postmeta IN-list query fired on every
/// archive page render:
///   SELECT * FROM wp_postmeta WHERE post_id IN (...) AND meta_key='...'
fn is_postmeta_in_query(sql: &str) -> bool {
    let lo = sql.to_ascii_lowercase();
    lo.contains("wp_postmeta") && lo.contains("post_id") && lo.contains(" in ")
}

/// One-shot: CREATE INDEX IF NOT EXISTS idx_postmeta_key_post ON
/// wp_postmeta(meta_key, post_id, meta_value)
/// Covering index turns the IN-list scan into an index-only lookup.
/// No-op if index already exists (IF NOT EXISTS). Safe to call concurrently.
fn ensure_postmeta_index(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_postmeta_key_post \
         ON wp_postmeta(meta_key, post_id, meta_value);",
    )
}
// ─────────────────────────────────────────────────────────────────────────────
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWrite;
use tracing::{debug, warn};

const RESULT_CACHE_CAP: usize = 4096;
const CACHE_TTL: Duration = Duration::from_millis(500);

#[derive(Clone)]
struct CachedResult {
    col_defs: Vec<(String, String)>,
    rows: Vec<Vec<String>>,
    inserted_at: Instant,
    epoch: u64,
}

type SharedCache = Arc<Mutex<LruCache<u64, CachedResult>>>;

/// Dedicated autoload cache — keyed by normalized SQL.
/// Cleared on any wp_options write; never evicted by LRU.
type AutoloadCache = Arc<Mutex<HashMap<String, CachedResult>>>;

pub struct SharedState {
    pub file: PathBuf,
    pub mode: String,
    pub cache: SharedCache,
    pub write_epoch: Arc<Mutex<u64>>,
    /// WP-specific: tracks whether the covering index on wp_postmeta has been
    /// created in this daemon lifetime. Set once on first postmeta IN-list query.
    /// Only applied when the database contains wp_ prefix tables.
    pub postmeta_index_created: Arc<AtomicBool>,
    /// WP autoload options — served from memory after first hit, invalidated
    /// on any INSERT/UPDATE/DELETE/REPLACE touching wp_options.
    pub autoload_cache: AutoloadCache,
}

pub fn new_shared_state(file: PathBuf, mode: String) -> Arc<SharedState> {
    Arc::new(SharedState {
        file,
        mode,
        cache: Arc::new(Mutex::new(LruCache::new(
            NonZeroUsize::new(RESULT_CACHE_CAP).unwrap(),
        ))),
        write_epoch: Arc::new(Mutex::new(0)),
        postmeta_index_created: Arc::new(AtomicBool::new(false)),
        autoload_cache: Arc::new(Mutex::new(HashMap::new())),
    })
}

pub struct SynapseMysqlAsync {
    state: Arc<SharedState>,
    /// Per-connection sqlite handle, lazily opened on first query in a blocking task.
    conn: Option<Arc<Mutex<Connection>>>,
    current_db: Option<String>,
    next_stmt_id: u32,
    prepared: HashMap<u32, String>,
    /// Last _found_rows value from SQL_CALC_FOUND_ROWS queries, per connection.
    last_found_rows: u64,
    /// Tracks whether a MySQL transaction is in progress (mapped to SQLite SAVEPOINT).
    in_tx: bool,
}

impl SynapseMysqlAsync {
    pub fn new(state: Arc<SharedState>) -> Self {
        Self {
            state,
            conn: None,
            current_db: None,
            next_stmt_id: 1,
            prepared: HashMap::new(),
            last_found_rows: 0,
            in_tx: false,
        }
    }

    /// Lazy-open per-connection rusqlite handle (read-mostly, WAL).
    async fn get_conn(&mut self) -> io::Result<Arc<Mutex<Connection>>> {
        if let Some(c) = &self.conn {
            return Ok(c.clone());
        }
        let file = self.state.file.clone();
        let conn = tokio::task::spawn_blocking(move || open_conn(&file))
            .await
            .map_err(io_other)??;
        let arc = Arc::new(Mutex::new(conn));
        self.conn = Some(arc.clone());
        Ok(arc)
    }
}

fn open_conn(file: &Path) -> io::Result<Connection> {
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }
    let conn = Connection::open(file).map_err(io_other)?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(io_other)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(io_other)?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)
        .map_err(io_other)?;
    conn.pragma_update(None, "temp_store", "MEMORY")
        .map_err(io_other)?;
    conn.pragma_update(None, "mmap_size", 268_435_456_i64)
        .map_err(io_other)?;
    conn.pragma_update(None, "cache_size", -65536_i64)
        .map_err(io_other)?;
    // Register REGEXP UDF so MySQL `col REGEXP 'pattern'` works in SQLite.
    conn.create_scalar_function(
        "REGEXP",
        2,
        rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC
            | rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        |ctx| -> rusqlite::Result<bool> {
            let pattern: String = ctx.get(0)?;
            let text: String = ctx.get(1)?;
            let re = regex::Regex::new(&pattern).map_err(|e| {
                rusqlite::Error::UserFunctionError(
                    format!("REGEXP: bad pattern: {e}").into(),
                )
            })?;
            Ok(re.is_match(&text))
        },
    )
    .map_err(io_other)?;
    Ok(conn)
}

fn io_other<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

#[async_trait]
impl<W: AsyncWrite + Send + Sync + Unpin> AsyncMysqlShim<W> for SynapseMysqlAsync {
    type Error = io::Error;

    fn version(&self) -> String {
        format!("8.0.30-synapse-{}", env!("CARGO_PKG_VERSION"))
    }

    fn default_auth_plugin(&self) -> &str {
        "caching_sha2_password"
    }

    async fn on_init<'a>(
        &'a mut self,
        database: &'a str,
        writer: InitWriter<'a, W>,
    ) -> io::Result<()> {
        self.current_db = Some(database.to_string());
        debug!("on_init: USE {}", database);
        writer.ok().await
    }

    async fn on_query<'a>(
        &'a mut self,
        sql: &'a str,
        results: QueryResultWriter<'a, W>,
    ) -> io::Result<()> {
        let t0 = Instant::now();
        // Phase 1 MVP: handle MySQL-specific session-init queries inline so
        // pymysql/JDBC drivers can complete handshake without errors.
        let trimmed = sql.trim_end_matches(';').trim();
        let upper = trimmed.to_uppercase();
        if upper.starts_with("SET ") || upper.starts_with("USE ") {
            return results.completed(opensrv_mysql::OkResponse::default()).await;
        }
        if upper == "BEGIN" || upper.starts_with("START TRANSACTION") {
            if self.in_tx {
                warn!("BEGIN within BEGIN: nested transactions not supported, ignoring");
                return results.completed(opensrv_mysql::OkResponse::default()).await;
            }
            let conn = self.get_conn().await?;
            tokio::task::spawn_blocking(move || -> io::Result<()> {
                conn.lock().execute("SAVEPOINT mysql_tx", []).map_err(io_other)?;
                Ok(())
            })
            .await
            .map_err(io_other)??;
            self.in_tx = true;
            return results.completed(opensrv_mysql::OkResponse::default()).await;
        }
        if upper == "COMMIT" {
            let conn = self.get_conn().await?;
            tokio::task::spawn_blocking(move || -> io::Result<()> {
                conn.lock().execute("RELEASE SAVEPOINT mysql_tx", []).map_err(io_other)?;
                Ok(())
            })
            .await
            .map_err(io_other)??;
            self.in_tx = false;
            return results.completed(opensrv_mysql::OkResponse::default()).await;
        }
        if upper == "ROLLBACK" {
            let conn = self.get_conn().await?;
            tokio::task::spawn_blocking(move || -> io::Result<()> {
                let c = conn.lock();
                c.execute("ROLLBACK TO SAVEPOINT mysql_tx", []).map_err(io_other)?;
                c.execute("RELEASE SAVEPOINT mysql_tx", []).map_err(io_other)?;
                Ok(())
            })
            .await
            .map_err(io_other)??;
            self.in_tx = false;
            return results.completed(opensrv_mysql::OkResponse::default()).await;
        }
        if upper.starts_with("SHOW ") {
            // minimal SHOW response — empty result set
            let writer = results.start(&[]).await?;
            return writer.finish().await;
        }
        if upper.starts_with("SELECT @@") || upper.starts_with("SELECT VERSION()") {
            // metadata query — return single dummy row
            let cols = vec![Column {
                table: String::new(),
                column: "v".to_string(),
                coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
                colflags: ColumnFlags::empty(),
                collen: 0,
            }];
            let mut writer = results.start(&cols).await?;
            writer.write_row(&["8.0.30-synapse"]).await?;
            return writer.finish().await;
        }
        // Phase 1 MVP: skip rewrite for everything else, pass through to SQLite.
        let rewritten = synapse_mysql::rewrite::rewrite(sql, &self.state.mode).unwrap_or_else(|_| sql.to_string());
        let _mode = &self.state.mode;

        // FOUND_ROWS() sentinel → return cached last_found_rows value
        if rewritten.contains("FOUND_ROWS_SENTINEL") {
            let found = self.last_found_rows;
            let cols = vec![Column {
                table: String::new(),
                column: "FOUND_ROWS()".to_string(),
                coltype: ColumnType::MYSQL_TYPE_LONGLONG,
                colflags: ColumnFlags::empty(),
                collen: 0,
            }];
            let mut writer = results.start(&cols).await?;
            writer.write_row(&[found.to_string()]).await?;
            return writer.finish().await;
        }

        // ── DML writes (INSERT / UPDATE / DELETE / REPLACE) ──────────────────────
        if is_dml_write(&rewritten) {
            let sql_owned = rewritten.clone();
            let conn = self.get_conn().await?;
            let (rows_affected, last_insert_id) =
                tokio::task::spawn_blocking(move || -> io::Result<(u64, u64)> {
                    let c = conn.lock();
                    c.execute(&sql_owned, []).map_err(io_other)?;
                    let affected = c.changes();
                    let last_id = c.last_insert_rowid() as u64;
                    Ok((affected, last_id))
                })
                .await
                .map_err(io_other)??;
            { *self.state.write_epoch.lock() += 1; }
            if is_wp_options_write(&rewritten) {
                self.state.autoload_cache.lock().clear();
                debug!("wp_options write — autoload cache invalidated");
            }
            debug!("DML {} → affected={} last_id={} ({}µs)",
                &rewritten[..rewritten.len().min(50)],
                rows_affected, last_insert_id,
                t0.elapsed().as_micros()
            );
            return results.completed(opensrv_mysql::OkResponse {
                affected_rows: rows_affected,
                last_insert_id,
                ..Default::default()
            }).await;
        }

        // ── wp_options autoload dedicated cache ───────────────────────────────
        // Any write to wp_options invalidates the autoload cache immediately.
        if is_wp_options_write(&rewritten) {
            self.state.autoload_cache.lock().clear();
            debug!("wp_options write — autoload cache invalidated");
        }
        // Autoload reads: serve from dedicated in-memory cache (target <0.1ms).
        if is_autoload_query(&rewritten) {
            let norm = rewritten.to_ascii_lowercase();
            let hit: Option<CachedResult> = self.state.autoload_cache.lock().get(&norm).cloned();
            if let Some(cached) = hit {
                debug!("autoload cache hit ({}µs)", t0.elapsed().as_micros());
                return write_cached(results, &cached).await;
            }
            // Cache miss — fall through to SQLite, then populate autoload cache below.
        }
        // ─────────────────────────────────────────────────────────────────────

        let key = blake3_u64(&rewritten);
        let current_epoch = { *self.state.write_epoch.lock() };

        // Cache check — drop lock guard before await
        let cache_hit: Option<CachedResult> = {
            let mut guard = self.state.cache.lock();
            guard.get(&key).cloned()
        };
        if let Some(cached) = cache_hit {
            if cached.epoch == current_epoch
                && cached.inserted_at.elapsed() < CACHE_TTL
            {
                debug!("cache hit {} ({}µs)", &rewritten[..rewritten.len().min(50)], t0.elapsed().as_micros());
                return write_cached(results, &cached).await;
            }
        }

        let conn = self.get_conn().await?;

        // WP optimization: lazily create a covering index on wp_postmeta the
        // first time we see a postmeta IN-list query. The index turns the
        // per-row IN scan (p50 ~100ms) into an index-only lookup (<2ms).
        // Guarded by an AtomicBool so we pay only one DDL round-trip per daemon
        // lifetime regardless of connection count. Only fires for WP databases
        // (query must reference wp_postmeta).
        if is_postmeta_in_query(&rewritten)
            && !self.state.postmeta_index_created.load(Ordering::Relaxed)
        {
            // Use compare_exchange to ensure a single writer wins the race.
            if self.state.postmeta_index_created
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                let idx_conn = conn.clone();
                tokio::task::spawn_blocking(move || {
                    let c = idx_conn.lock();
                    if let Err(e) = ensure_postmeta_index(&c) {
                        warn!("wp_postmeta index creation failed: {}", e);
                    } else {
                        debug!("wp_postmeta covering index ensured (idx_postmeta_key_post)");
                    }
                })
                .await
                .map_err(io_other)?;
            }
        }

        let sql_owned = rewritten.clone();
        let exec = tokio::task::spawn_blocking(move || -> io::Result<(Vec<(String, String)>, Vec<Vec<String>>)> {
            let conn = conn.lock();
            execute_select(&conn, &sql_owned).map_err(io_other)
        })
        .await
        .map_err(io_other)??;

        // Track _found_rows value for subsequent SELECT FOUND_ROWS() calls
        if let Some(idx) = exec.0.iter().position(|(name, _)| name == "_found_rows") {
            if let Some(first_row) = exec.1.first() {
                if let Some(val) = first_row.get(idx) {
                    self.last_found_rows = val.parse().unwrap_or(0);
                }
            }
        }

        let cached = CachedResult {
            col_defs: exec.0,
            rows: exec.1,
            inserted_at: Instant::now(),
            epoch: current_epoch,
        };
        {
            self.state.cache.lock().put(key, cached.clone());
        }
        // Populate autoload cache on first SQLite hit.
        if is_autoload_query(&rewritten) {
            let norm = rewritten.to_ascii_lowercase();
            self.state.autoload_cache.lock().insert(norm, cached.clone());
            debug!("autoload cache populated ({} rows)", cached.rows.len());
        }

        debug!("query {} → {} rows ({}µs)",
            &rewritten[..rewritten.len().min(50)],
            cached.rows.len(),
            t0.elapsed().as_micros()
        );

        write_cached(results, &cached).await
    }

    async fn on_prepare<'a>(
        &'a mut self,
        _query: &'a str,
        info: StatementMetaWriter<'a, W>,
    ) -> io::Result<()> {
        // Phase 1 MVP: minimal prepare reply with stmt_id, no params, no result fields.
        let id = self.next_stmt_id;
        self.next_stmt_id += 1;
        self.prepared.insert(id, _query.to_string());
        info.reply(id, &[], &[]).await
    }

    async fn on_execute<'a>(
        &'a mut self,
        stmt_id: u32,
        params: ParamParser<'a>,
        results: QueryResultWriter<'a, W>,
    ) -> io::Result<()> {
        let sql = self
            .prepared
            .get(&stmt_id)
            .cloned()
            .unwrap_or_else(|| "SELECT 1".to_string());

        // Collect bound parameter values from the binary protocol.
        let mut values: Vec<rusqlite::types::Value> = vec![];
        for pv in params {
            let v = match pv.value.into_inner() {
                ValueInner::NULL => rusqlite::types::Value::Null,
                ValueInner::Int(i) => rusqlite::types::Value::Integer(i),
                ValueInner::UInt(u) => rusqlite::types::Value::Integer(u as i64),
                ValueInner::Double(d) => rusqlite::types::Value::Real(d),
                ValueInner::Bytes(b) => {
                    rusqlite::types::Value::Text(String::from_utf8_lossy(b).into_owned())
                }
                // Date/Time/Datetime: convert bytes to string representation
                ValueInner::Date(b) | ValueInner::Time(b) | ValueInner::Datetime(b) => {
                    rusqlite::types::Value::Text(String::from_utf8_lossy(b).into_owned())
                }
            };
            values.push(v);
        }

        let rewritten = synapse_mysql::rewrite::rewrite(&sql, &self.state.mode)
            .unwrap_or_else(|_| sql.clone());

        // DML path
        if is_dml_write(&rewritten) {
            let sql_owned = rewritten.clone();
            let conn = self.get_conn().await?;
            let (rows_affected, last_insert_id) =
                tokio::task::spawn_blocking(move || -> io::Result<(u64, u64)> {
                    let c = conn.lock();
                    c.execute(
                        &sql_owned,
                        rusqlite::params_from_iter(values.iter()),
                    )
                    .map_err(io_other)?;
                    let affected = c.changes();
                    let last_id = c.last_insert_rowid() as u64;
                    Ok((affected, last_id))
                })
                .await
                .map_err(io_other)??;
            { *self.state.write_epoch.lock() += 1; }
            if is_wp_options_write(&rewritten) {
                self.state.autoload_cache.lock().clear();
            }
            return results.completed(opensrv_mysql::OkResponse {
                affected_rows: rows_affected,
                last_insert_id,
                ..Default::default()
            }).await;
        }

        // SELECT path
        let conn = self.get_conn().await?;
        let exec = tokio::task::spawn_blocking(move || -> io::Result<(Vec<(String, String)>, Vec<Vec<String>>)> {
            let conn = conn.lock();
            execute_select_with_params(&conn, &rewritten, &values).map_err(io_other)
        })
        .await
        .map_err(io_other)??;
        let cached = CachedResult {
            col_defs: exec.0,
            rows: exec.1,
            inserted_at: Instant::now(),
            epoch: 0,
        };
        write_cached(results, &cached).await
    }

    async fn on_close(&mut self, stmt_id: u32) {
        self.prepared.remove(&stmt_id);
    }
}

fn execute_select(
    conn: &Connection,
    sql: &str,
) -> Result<(Vec<(String, String)>, Vec<Vec<String>>), rusqlite::Error> {
    let mut stmt = conn.prepare(sql)?;
    let col_count = stmt.column_count();
    let col_defs: Vec<(String, String)> = (0..col_count)
        .map(|i| {
            let name = stmt.column_name(i).unwrap_or("?").to_string();
            let decl = "TEXT".to_string();
            (name, decl)
        })
        .collect();

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut q = stmt.query([])?;
    while let Some(row) = q.next()? {
        let mut r = Vec::with_capacity(col_count);
        for i in 0..col_count {
            let val: rusqlite::types::Value = row.get(i)?;
            r.push(match val {
                rusqlite::types::Value::Null => String::from(""),
                rusqlite::types::Value::Integer(n) => n.to_string(),
                rusqlite::types::Value::Real(f) => f.to_string(),
                rusqlite::types::Value::Text(s) => s,
                rusqlite::types::Value::Blob(b) => format!("0x{}", hex::encode(&b)),
            });
        }
        rows.push(r);
    }
    Ok((col_defs, rows))
}

fn execute_select_with_params(
    conn: &Connection,
    sql: &str,
    params: &[rusqlite::types::Value],
) -> Result<(Vec<(String, String)>, Vec<Vec<String>>), rusqlite::Error> {
    let mut stmt = conn.prepare(sql)?;
    let col_count = stmt.column_count();
    let col_defs: Vec<(String, String)> = stmt
        .columns()
        .into_iter()
        .map(|c| {
            let name = c.name().to_string();
            let decl = c.decl_type().unwrap_or("TEXT").to_string();
            (name, decl)
        })
        .collect();

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut q = stmt.query(rusqlite::params_from_iter(params.iter()))?;
    while let Some(row) = q.next()? {
        let mut r = Vec::with_capacity(col_count);
        for i in 0..col_count {
            let val: rusqlite::types::Value = row.get(i)?;
            r.push(match val {
                rusqlite::types::Value::Null => String::from(""),
                rusqlite::types::Value::Integer(n) => n.to_string(),
                rusqlite::types::Value::Real(f) => f.to_string(),
                rusqlite::types::Value::Text(s) => s,
                rusqlite::types::Value::Blob(b) => format!("0x{}", hex::encode(&b)),
            });
        }
        rows.push(r);
    }
    Ok((col_defs, rows))
}

async fn write_cached<'a, W: AsyncWrite + Send + Sync + Unpin>(
    results: QueryResultWriter<'a, W>,
    cached: &CachedResult,
) -> io::Result<()> {
    let columns: Vec<Column> = cached
        .col_defs
        .iter()
        .map(|(name, decl)| Column {
            table: String::new(),
            column: name.clone(),
            coltype: map_decl_to_mysql(decl),
            colflags: ColumnFlags::empty(),
            collen: 0,
        })
        .collect();
    let mut writer = results.start(&columns).await?;
    for row in &cached.rows {
        writer.write_row(row).await?;
    }
    writer.finish().await
}

fn map_decl_to_mysql(decl: &str) -> ColumnType {
    let lo = decl.to_lowercase();
    if lo.contains("int") {
        ColumnType::MYSQL_TYPE_LONGLONG
    } else if lo.contains("real") || lo.contains("float") || lo.contains("double") {
        ColumnType::MYSQL_TYPE_DOUBLE
    } else if lo.contains("blob") {
        ColumnType::MYSQL_TYPE_BLOB
    } else {
        ColumnType::MYSQL_TYPE_VAR_STRING
    }
}

fn blake3_u64(s: &str) -> u64 {
    let h = blake3::hash(s.as_bytes());
    let b = h.as_bytes();
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

// hex used in execute_select for BLOB → text rendering
mod hex {
    pub fn encode(b: &[u8]) -> String {
        let mut s = String::with_capacity(b.len() * 2);
        for byte in b {
            s.push_str(&format!("{:02x}", byte));
        }
        s
    }
}
