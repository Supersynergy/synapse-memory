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
use std::collections::{HashMap, VecDeque};
use std::io;
use std::num::NonZeroUsize;

// ── Connection pool ───────────────────────────────────────────────────────────
pub const DEFAULT_POOL_SIZE: usize = 32;

/// Shared pool of reusable, pre-opened SQLite connections.
/// Each entry is an Arc<Mutex<Connection>> so it can be passed into spawn_blocking.
pub type ConnPool = Arc<Mutex<VecDeque<Arc<Mutex<Connection>>>>>;

// ── wp_options autoload cache ─────────────────────────────────────────────────
const AUTOLOAD_PATTERNS: &[&str] = &[
    "select option_value from wp_options where autoload",
    "select option_name, option_value from wp_options where autoload",
    "select * from wp_options where autoload",
];

/// Count `?` placeholders in SQL, ignoring those inside single/double-quoted
/// string literals or after `--` line comments. Good enough for session-init
/// and WordPress queries (no doc-strings, no q-quotes).
fn count_placeholders(sql: &str) -> usize {
    let bytes = sql.as_bytes();
    let mut n = 0usize;
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_back = false;
    while i < bytes.len() {
        let b = bytes[i];
        // Skip dollar-quoted token boundaries (e.g. PostgreSQL-style $$body$$).
        // Outside of any quoted region, treat `$$` as a no-op pair so the bytes
        // inside cannot be miscounted as `?` placeholders.
        if !in_single && !in_double && !in_back
            && b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'$'
        {
            i += 2;
            continue;
        }
        if !in_double && !in_back && b == b'\'' {
            // toggle, accounting for escaped quote ''
            if in_single && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                i += 2;
                continue;
            }
            in_single = !in_single;
        } else if !in_single && !in_back && b == b'"' {
            in_double = !in_double;
        } else if !in_single && !in_double && b == b'`' {
            in_back = !in_back;
        } else if b == b'\\' && (in_single || in_double) {
            i += 2;
            continue;
        } else if !in_single && !in_double && !in_back && b == b'?' {
            n += 1;
        } else if !in_single && !in_double && !in_back
            && b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-'
        {
            // line comment
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    n
}

fn is_autoload_query(sql: &str) -> bool {
    let lo = sql.to_ascii_lowercase();
    AUTOLOAD_PATTERNS.iter().any(|p| lo.contains(p))
}

/// Fast byte-level scan: if the SQL contains any MySQL-ism that the rewrite
/// layer cares about (backticks, MySQL-only funcs, CALC_FOUND_ROWS, NOW(),
/// ON DUPLICATE KEY, etc.) trigger the full rewriter. Otherwise pass through.
fn needs_rewrite(sql: &str) -> bool {
    if sql.as_bytes().contains(&b'`') {
        return true;
    }
    let upper = sql.trim_start().to_ascii_uppercase();
    // DDL must always go through the rewriter — SQLite syntax differs significantly.
    if upper.starts_with("CREATE ") || upper.starts_with("DROP ") || upper.starts_with("ALTER ")
        || upper.starts_with("TRUNCATE ") || upper.starts_with("RENAME ")
        || upper.starts_with("LOCK ") || upper.starts_with("UNLOCK ")
        || upper.starts_with("ANALYZE ") || upper.starts_with("GRANT ")
        || upper.starts_with("REPLACE ") || upper.starts_with("INSERT IGNORE ")
    {
        return true;
    }
    const TOKENS: &[&str] = &[
        "SQL_CALC_FOUND_ROWS",
        "FOUND_ROWS()",
        "NOW(",
        "UNIX_TIMESTAMP",
        "GROUP_CONCAT",
        "STR_TO_DATE",
        "DATE_FORMAT",
        "ON DUPLICATE KEY",
        "REGEXP",
        "AUTO_INCREMENT",
        "UNSIGNED",
        "ENGINE=",
        "ENGINE =",
        "DEFAULT CHARSET",
        "COLLATE",
        "LONGTEXT",
        "MEDIUMTEXT",
        "TINYTEXT",
    ];
    TOKENS.iter().any(|t| upper.contains(t))
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
    /// Shared pool of reusable SQLite connections.
    pub conn_pool: ConnPool,
    pub pool_max: usize,
}

pub fn new_shared_state(file: PathBuf, mode: String) -> Arc<SharedState> {
    new_shared_state_with_pool(file, mode, DEFAULT_POOL_SIZE)
}

pub fn new_shared_state_with_pool(file: PathBuf, mode: String, pool_size: usize) -> Arc<SharedState> {
    Arc::new(SharedState {
        file,
        mode,
        cache: Arc::new(Mutex::new(LruCache::new(
            NonZeroUsize::new(RESULT_CACHE_CAP).unwrap(),
        ))),
        write_epoch: Arc::new(Mutex::new(0)),
        postmeta_index_created: Arc::new(AtomicBool::new(false)),
        autoload_cache: Arc::new(Mutex::new(HashMap::new())),
        conn_pool: Arc::new(Mutex::new(VecDeque::new())),
        pool_max: pool_size,
    })
}

pub struct SynapseMysqlAsync {
    state: Arc<SharedState>,
    /// Checked-out connection from the shared pool (returned to pool on shim drop).
    conn: Option<Arc<Mutex<Connection>>>,
    current_db: Option<String>,
    next_stmt_id: u32,
    prepared: HashMap<u32, String>,
    /// Last _found_rows value from SQL_CALC_FOUND_ROWS queries, per connection.
    last_found_rows: u64,
    /// Last insert rowid for SELECT LAST_INSERT_ID(), per connection.
    last_insert_id: u64,
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
            last_insert_id: 0,
            in_tx: false,
        }
    }

    /// Returns a connection from the shared pool, or opens a new one.
    /// The connection is held for the lifetime of this shim and returned to the pool on drop.
    async fn get_conn(&mut self) -> io::Result<Arc<Mutex<Connection>>> {
        if let Some(c) = &self.conn {
            return Ok(c.clone());
        }
        // Try to get one from the pool first.
        let pooled = self.state.conn_pool.lock().pop_front();
        let arc = if let Some(c) = pooled {
            c
        } else {
            let file = self.state.file.clone();
            let conn = tokio::task::spawn_blocking(move || open_conn(&file))
                .await
                .map_err(io_other)??;
            Arc::new(Mutex::new(conn))
        };
        self.conn = Some(arc.clone());
        Ok(arc)
    }
}

impl Drop for SynapseMysqlAsync {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take() {
            let mut guard = self.state.conn_pool.lock();
            if guard.len() < self.state.pool_max {
                guard.push_back(c);
            }
            // if pool is full, the Arc is dropped and the connection closes
        }
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
    conn.pragma_update(None, "mmap_size", 536_870_912_i64)
        .map_err(io_other)?;
    conn.pragma_update(None, "cache_size", -65536_i64)
        .map_err(io_other)?;
    // Bump rusqlite's per-connection prepared-statement LRU from default 16 → 256.
    // Hot OLTP point-select reuses parsed plans across many distinct SQL keys
    // (sysbench oltp_point_select uses a single template, but mixed workloads
    // benefit from larger headroom).
    conn.set_prepared_statement_cache_capacity(256);
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

fn extract_table_name_from_show_columns(upper_sql: &str) -> String {
    // Handles: SHOW [FULL] COLUMNS FROM `table` or table
    let s = upper_sql
        .replace("SHOW FULL COLUMNS FROM", "")
        .replace("SHOW COLUMNS FROM", "");
    s.trim().trim_matches('`').trim_matches('\'').trim_matches('"').to_string()
}

fn io_other<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

#[async_trait]
impl<W: AsyncWrite + Send + Sync + Unpin> AsyncMysqlShim<W> for SynapseMysqlAsync {
    type Error = io::Error;

    fn version(&self) -> String {
        format!("8.0.30-synapse-{}", env!("CARGO_PKG_VERSION"))
    }

    fn default_auth_plugin(&self) -> &str {
        // mysql_native_password is the most compatible plugin for PHP mysqli
        // and the wp-cli toolchain. caching_sha2_password requires a public-key
        // RSA handshake that opensrv-mysql does not implement without TLS,
        // which manifested as "Error establishing a database connection" from wpdb.
        "mysql_native_password"
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
        // Hot path: keep at debug! to avoid string-format + tracing dispatch on every query.
        // At 1k+ OPS the info! cost dominates total query budget.
        debug!("on_query: {}", sql);
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
        if upper.contains("SELECT DATABASE()") {
            let cols = vec![Column {
                table: String::new(),
                column: "DATABASE()".to_string(),
                coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
                colflags: ColumnFlags::empty(),
                collen: 0,
            }];
            let mut writer = results.start(&cols).await?;
            let db_name = self.current_db.clone().unwrap_or_else(|| "wordpress".to_string());
            writer.write_row(&[db_name.as_str()]).await?;
            return writer.finish().await;
        }
        if upper.starts_with("SHOW FULL COLUMNS FROM") || upper.starts_with("SHOW COLUMNS FROM") {
            let table = extract_table_name_from_show_columns(&upper);
            let conn = self.get_conn().await?;
            let table_clone = table.clone();
            let rows = tokio::task::spawn_blocking(move || -> io::Result<Vec<Vec<String>>> {
                let c = conn.lock();
                let pragma_sql = format!("PRAGMA table_info(\"{}\")", table_clone);
                let mut stmt = c.prepare(&pragma_sql).map_err(io_other)?;
                let _col_count = stmt.column_count();
                let mut rows = Vec::new();
                let mut q = stmt.query([]).map_err(io_other)?;
                while let Some(row) = q.next().map_err(io_other)? {
                    // PRAGMA table_info cols: cid, name, type, notnull, dflt_value, pk
                    let name: String = row.get(1).unwrap_or_default();
                    let col_type: String = row.get(2).unwrap_or_else(|_| "text".to_string());
                    let notnull: i64 = row.get(3).unwrap_or(0);
                    let dflt: String = row.get(4).unwrap_or_default();
                    let pk: i64 = row.get(5).unwrap_or(0);
                    let null_str = if notnull == 0 { "YES" } else { "NO" };
                    let key_str = if pk == 1 { "PRI" } else { "" };
                    let extra_str = if pk == 1 { "auto_increment" } else { "" };
                    // Field, Type, Collation, Null, Key, Default, Extra, Privileges, Comment
                    rows.push(vec![
                        name,
                        col_type.to_lowercase(),
                        String::new(),
                        null_str.to_string(),
                        key_str.to_string(),
                        dflt,
                        extra_str.to_string(),
                        "select,insert,update,references".to_string(),
                        String::new(),
                    ]);
                }
                Ok(rows)
            })
            .await
            .map_err(io_other)??;

            let cols = vec![
                Column { table: String::new(), column: "Field".to_string(), coltype: ColumnType::MYSQL_TYPE_VAR_STRING, colflags: ColumnFlags::empty(), collen: 0 },
                Column { table: String::new(), column: "Type".to_string(), coltype: ColumnType::MYSQL_TYPE_VAR_STRING, colflags: ColumnFlags::empty(), collen: 0 },
                Column { table: String::new(), column: "Collation".to_string(), coltype: ColumnType::MYSQL_TYPE_VAR_STRING, colflags: ColumnFlags::empty(), collen: 0 },
                Column { table: String::new(), column: "Null".to_string(), coltype: ColumnType::MYSQL_TYPE_VAR_STRING, colflags: ColumnFlags::empty(), collen: 0 },
                Column { table: String::new(), column: "Key".to_string(), coltype: ColumnType::MYSQL_TYPE_VAR_STRING, colflags: ColumnFlags::empty(), collen: 0 },
                Column { table: String::new(), column: "Default".to_string(), coltype: ColumnType::MYSQL_TYPE_VAR_STRING, colflags: ColumnFlags::empty(), collen: 0 },
                Column { table: String::new(), column: "Extra".to_string(), coltype: ColumnType::MYSQL_TYPE_VAR_STRING, colflags: ColumnFlags::empty(), collen: 0 },
                Column { table: String::new(), column: "Privileges".to_string(), coltype: ColumnType::MYSQL_TYPE_VAR_STRING, colflags: ColumnFlags::empty(), collen: 0 },
                Column { table: String::new(), column: "Comment".to_string(), coltype: ColumnType::MYSQL_TYPE_VAR_STRING, colflags: ColumnFlags::empty(), collen: 0 },
            ];
            let mut writer = results.start(&cols).await?;
            for row in &rows {
                writer.write_row(row).await?;
            }
            return writer.finish().await;
        }
        if upper.starts_with("SHOW TABLES") {
            let conn = self.get_conn().await?;
            let rows = tokio::task::spawn_blocking(move || -> io::Result<Vec<String>> {
                let c = conn.lock();
                let mut stmt = c.prepare(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_mysql_%' ORDER BY name"
                ).map_err(io_other)?;
                let mut q = stmt.query([]).map_err(io_other)?;
                let mut out = Vec::new();
                while let Some(r) = q.next().map_err(io_other)? {
                    out.push(r.get::<_,String>(0).unwrap_or_default());
                }
                Ok(out)
            }).await.map_err(io_other)??;
            let cols = vec![Column {
                table: String::new(),
                column: "Tables_in_database".to_string(),
                coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
                colflags: ColumnFlags::empty(),
                collen: 0,
            }];
            let mut writer = results.start(&cols).await?;
            for r in &rows { writer.write_row(&[r.as_str()]).await?; }
            return writer.finish().await;
        }
        if upper.starts_with("SHOW DATABASES") {
            let cols = vec![Column {
                table: String::new(),
                column: "Database".to_string(),
                coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
                colflags: ColumnFlags::empty(),
                collen: 0,
            }];
            let mut writer = results.start(&cols).await?;
            let db = self.current_db.clone().unwrap_or_else(|| "wordpress".to_string());
            writer.write_row(&[db.as_str()]).await?;
            return writer.finish().await;
        }
        if upper.starts_with("SHOW ") {
            // minimal SHOW response — empty result set
            let writer = results.start(&[]).await?;
            return writer.finish().await;
        }
        // mysql 9.x CLI sends `select $$` as a delimiter probe; real MySQL
        // returns a syntax error. We mirror that to keep CLI happy.
        if upper == "SELECT $$" || upper.starts_with("SELECT $$ ") {
            return results.error(
                ErrorKind::ER_PARSE_ERROR,
                b"You have an error in your SQL syntax near '$$'",
            ).await;
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
        // Fast-path: plain SELECT/INSERT/UPDATE/DELETE without MySQL-isms can skip rewrite entirely.
        // Detection is byte-level, no regex, no allocation.
        let rewritten = if needs_rewrite(sql) {
            synapse_mysql::rewrite::rewrite(sql, &self.state.mode).unwrap_or_else(|_| sql.to_string())
        } else {
            sql.to_string()
        };
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

        // LAST_INSERT_ID() sentinel
        if upper.contains("LAST_INSERT_ID()") {
            let last_id = self.last_insert_id;
            let cols = vec![Column {
                table: String::new(),
                column: "LAST_INSERT_ID()".to_string(),
                coltype: ColumnType::MYSQL_TYPE_LONGLONG,
                colflags: ColumnFlags::empty(),
                collen: 0,
            }];
            let mut writer = results.start(&cols).await?;
            writer.write_row(&[last_id.to_string()]).await?;
            return writer.finish().await;
        }

        // ── DDL (CREATE / DROP / ALTER / TRUNCATE / RENAME / GRANT / ANALYZE) ─
        // MySQL DDL has dialects SQLite cannot parse (CREATE DATABASE,
        // BIGINT(20) UNSIGNED, ENGINE=InnoDB, KEY foo (col), …). Translate via
        // rewrite_ddl which returns 1+ statements (CREATE TABLE may emit
        // accompanying CREATE INDEX statements for inline KEY clauses).
        if synapse_mysql::rewrite::is_ddl(sql) {
            let stmts = synapse_mysql::rewrite::rewrite_ddl(sql)
                .unwrap_or_else(|_| vec![rewritten.clone()]);
            let conn = self.get_conn().await?;
            tokio::task::spawn_blocking(move || -> io::Result<()> {
                let c = conn.lock();
                for s in &stmts {
                    let trimmed = s.trim();
                    if trimmed.is_empty() { continue; }
                    c.execute_batch(trimmed).map_err(io_other)?;
                }
                Ok(())
            })
            .await
            .map_err(io_other)??;
            { *self.state.write_epoch.lock() += 1; }
            debug!("DDL ok ({}µs)", t0.elapsed().as_micros());
            return results.completed(opensrv_mysql::OkResponse::default()).await;
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
            self.last_insert_id = last_insert_id;
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
        query: &'a str,
        info_writer: StatementMetaWriter<'a, W>,
    ) -> io::Result<()> {
        // Phase 1 MVP: minimal prepare reply with stmt_id, no params, no result fields.
        let id = self.next_stmt_id;
        self.next_stmt_id += 1;
        debug!("on_prepare id={}: {}", id, query);
        self.prepared.insert(id, query.to_string());
        // Count `?` placeholders so opensrv emits matching number of param defs.
        // Without this, the client sends N params but we declared 0 → client mismatch.
        // We also want 0-param fallback so `mysql` CLI's session-init queries
        // (which contain no `?`) pass through cleanly.
        let n_params = count_placeholders(query);
        let param_def = Column {
            table: String::new(),
            column: "?".to_string(),
            coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
            colflags: ColumnFlags::empty(),
            collen: 0,
        };
        let params: Vec<Column> = (0..n_params).map(|_| param_def.clone()).collect();
        info_writer.reply(id, &params, &[]).await
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
        debug!("on_execute id={}: {}", stmt_id, sql);

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
            self.last_insert_id = last_insert_id;
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
    // Per-connection prepared statement cache (rusqlite LRU, default 16 entries).
    // Hot point-select paths reuse the parsed plan → ~1.7-2× OPS on OLTP RO.
    let mut stmt = conn.prepare_cached(sql)?;
    let col_count = stmt.column_count();
    let col_defs: Vec<(String, String)> = (0..col_count)
        .map(|i| {
            let name = stmt.column_name(i).unwrap_or("?").to_string();
            let decl = "TEXT".to_string();
            (name, decl)
        })
        .collect();

    // SQLite treats `?`/`?N`/`:name`/`@name`/`$name` as bind placeholders.
    // The MySQL `mysql` CLI session-init sends queries like `select $$` which
    // SQLite parses as a single `$` placeholder. Without a real bind value
    // SQLite returns "Wrong number of parameters". For text-protocol passthrough
    // we have no bound params to give, so bind NULL for each declared placeholder
    // and let the query run. Behaviour matches a no-op session-init probe.
    let n_params = stmt.parameter_count();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let nulls: Vec<rusqlite::types::Value> =
        (0..n_params).map(|_| rusqlite::types::Value::Null).collect();
    let mut q = stmt.query(rusqlite::params_from_iter(nulls.iter()))?;
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
    let mut stmt = conn.prepare_cached(sql)?;
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

#[cfg(test)]
mod tests {
    use super::count_placeholders;

    #[test]
    fn count_placeholders_basic() {
        assert_eq!(count_placeholders("SELECT 1"), 0);
        assert_eq!(count_placeholders("SELECT ?"), 1);
        assert_eq!(count_placeholders("SELECT ?, ?, ?"), 3);
    }

    #[test]
    fn count_placeholders_skips_dollar_dollar() {
        // Per HIGH-2: `$$` token boundary must be skipped so it cannot
        // be misread as anything placeholder-bearing.
        assert_eq!(count_placeholders("select $$"), 0);
        // After a `$$` opener and a matching `$$` closer, normal scanning
        // resumes; the `?` outside is counted.
        assert_eq!(count_placeholders("select $$ $$ ?"), 1);
    }

    #[test]
    fn count_placeholders_skips_strings_and_comments() {
        assert_eq!(count_placeholders("SELECT '?'"), 0);
        assert_eq!(count_placeholders("SELECT \"?\""), 0);
        assert_eq!(count_placeholders("SELECT 1 -- ?\n , ?"), 1);
    }
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
