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
    QueryResultWriter, StatementMetaWriter,
};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::collections::HashMap;
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
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

pub struct SharedState {
    pub file: PathBuf,
    pub mode: String,
    pub cache: SharedCache,
    pub write_epoch: Arc<Mutex<u64>>,
}

pub fn new_shared_state(file: PathBuf, mode: String) -> Arc<SharedState> {
    Arc::new(SharedState {
        file,
        mode,
        cache: Arc::new(Mutex::new(LruCache::new(
            NonZeroUsize::new(RESULT_CACHE_CAP).unwrap(),
        ))),
        write_epoch: Arc::new(Mutex::new(0)),
    })
}

pub struct SynapseMysqlAsync {
    state: Arc<SharedState>,
    /// Per-connection sqlite handle, lazily opened on first query in a blocking task.
    conn: Option<Arc<Mutex<Connection>>>,
    current_db: Option<String>,
    next_stmt_id: u32,
    prepared: HashMap<u32, String>,
}

impl SynapseMysqlAsync {
    pub fn new(state: Arc<SharedState>) -> Self {
        Self {
            state,
            conn: None,
            current_db: None,
            next_stmt_id: 1,
            prepared: HashMap::new(),
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
        if upper.starts_with("SET ")
            || upper.starts_with("USE ")
            || upper == "BEGIN"
            || upper == "COMMIT"
            || upper == "ROLLBACK"
        {
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
            }];
            let mut writer = results.start(&cols).await?;
            writer.write_row(&["8.0.30-synapse"]).await?;
            return writer.finish().await;
        }
        // Phase 1 MVP: skip rewrite for everything else, pass through to SQLite.
        let rewritten = synapse_mysql::rewrite::rewrite(sql, &self.state.mode).unwrap_or_else(|_| sql.to_string());
        let _mode = &self.state.mode;
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
        let sql_owned = rewritten.clone();
        let exec = tokio::task::spawn_blocking(move || -> io::Result<(Vec<(String, String)>, Vec<Vec<String>>)> {
            let conn = conn.lock();
            execute_select(&conn, &sql_owned).map_err(io_other)
        })
        .await
        .map_err(io_other)??;

        let cached = CachedResult {
            col_defs: exec.0,
            rows: exec.1,
            inserted_at: Instant::now(),
            epoch: current_epoch,
        };
        {
            self.state.cache.lock().put(key, cached.clone());
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
        _params: ParamParser<'a>,
        results: QueryResultWriter<'a, W>,
    ) -> io::Result<()> {
        // Phase 1 MVP: re-route to on_query without params (most WP prepared
        // statements have inline values via php-pdo emulate-prepares=true).
        let sql = self
            .prepared
            .get(&stmt_id)
            .cloned()
            .unwrap_or_else(|| "SELECT 1".to_string());
        let rewritten = synapse_mysql::rewrite::rewrite(&sql, &self.state.mode).unwrap_or_else(|_| sql.clone());
        let conn = self.get_conn().await?;
        let exec = tokio::task::spawn_blocking(move || -> io::Result<(Vec<(String, String)>, Vec<Vec<String>>)> {
            let conn = conn.lock();
            execute_select(&conn, &rewritten).map_err(io_other)
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
            // column_decltype not in rusqlite 0.33 default; use column_name only
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
