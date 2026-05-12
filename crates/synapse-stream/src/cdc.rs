use anyhow::Result;
use async_stream::try_stream;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::Stream;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub ts: i64,
    pub op: Op,
    pub table: String,
    pub row: Value,
}

struct QueuedEvent {
    ts: i64,
    op: &'static str,
    table: String,
    row_json: String,
}

pub struct CdcWriter {
    tx: mpsc::Sender<QueuedEvent>,
}

impl CdcWriter {
    /// Send an event into the in-memory queue (non-blocking, ~100ns).
    pub fn emit(&self, op: Op, table: &str, row: Value) -> Result<()> {
        let ts = Utc::now().timestamp_millis();
        let op_str = match op {
            Op::Insert => "insert",
            Op::Update => "update",
            Op::Delete => "delete",
        };
        let ev = QueuedEvent {
            ts,
            op: op_str,
            table: table.to_string(),
            row_json: serde_json::to_string(&row)?,
        };
        // try_send → never block hot path; drop if queue full (back-pressure)
        let _ = self.tx.try_send(ev);
        Ok(())
    }
}

pub struct CdcReader {
    pub db_path: PathBuf,
    pub last_position: i64,
    writer: Arc<CdcWriter>,
}

const BATCH_SIZE: usize = 1000;
const FLUSH_MS: u64 = 100;
const QUEUE_CAP: usize = 10_000;

impl CdcReader {
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self> {
        let db_path = db_path.into();
        let conn = Connection::open(&db_path)?;
        Self::ensure_schema(&conn)?;
        drop(conn);

        let (tx, rx) = mpsc::channel::<QueuedEvent>(QUEUE_CAP);
        let writer = Arc::new(CdcWriter { tx });

        // Spawn background flush task
        let flush_path = db_path.clone();
        tokio::spawn(async move {
            flush_loop(flush_path, rx).await;
        });

        Ok(Self { db_path, last_position: 0, writer })
    }

    fn ensure_schema(conn: &Connection) -> Result<()> {
        // WAL mode → concurrent readers never block writer
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _cdc_log (
                id       INTEGER PRIMARY KEY AUTOINCREMENT,
                ts       INTEGER NOT NULL,
                op       TEXT    NOT NULL,
                tbl      TEXT    NOT NULL,
                row_json TEXT    NOT NULL
            );
            CREATE TABLE IF NOT EXISTS _cdc_queue (
                id       INTEGER PRIMARY KEY AUTOINCREMENT,
                ts       INTEGER NOT NULL,
                op       TEXT    NOT NULL,
                tbl      TEXT    NOT NULL,
                row_json TEXT    NOT NULL,
                inserted_at INTEGER NOT NULL DEFAULT (unixepoch('now','subsec')*1000)
            );",
        )?;
        Ok(())
    }

    /// Install CDC triggers on `table`. Triggers write to `_cdc_queue`.
    /// The flush task moves rows from queue → `_cdc_log` in batches.
    pub fn install_triggers(&self, table: &str) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();

        let json_obj = |prefix: &str| -> String {
            cols.iter()
                .map(|c| format!("'{}', {}.{}", c, prefix, c))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let new_obj = json_obj("NEW");
        let old_obj = json_obj("OLD");

        conn.execute_batch(&format!(
            "
            CREATE TRIGGER IF NOT EXISTS _cdc_{t}_insert
            AFTER INSERT ON {t} BEGIN
                INSERT INTO _cdc_queue(ts,op,tbl,row_json)
                VALUES(unixepoch('now','subsec')*1000,'insert','{t}',json_object({new}));
            END;
            CREATE TRIGGER IF NOT EXISTS _cdc_{t}_update
            AFTER UPDATE ON {t} BEGIN
                INSERT INTO _cdc_queue(ts,op,tbl,row_json)
                VALUES(unixepoch('now','subsec')*1000,'update','{t}',json_object({new}));
            END;
            CREATE TRIGGER IF NOT EXISTS _cdc_{t}_delete
            AFTER DELETE ON {t} BEGIN
                INSERT INTO _cdc_queue(ts,op,tbl,row_json)
                VALUES(unixepoch('now','subsec')*1000,'delete','{t}',json_object({old}));
            END;
            ",
            t = table,
            new = new_obj,
            old = old_obj,
        ))?;
        Ok(())
    }

    pub fn tail(&mut self) -> Pin<Box<dyn Stream<Item = Result<ChangeEvent>> + Send + '_>> {
        let db_path = self.db_path.clone();
        Box::pin(try_stream! {
            loop {
                let rows = Self::poll_rows(&db_path, self.last_position)?;
                let found = rows.len();
                for (id, ts, op_str, table, row_json) in rows {
                    let op = match op_str.as_str() {
                        "insert" => Op::Insert,
                        "update" => Op::Update,
                        "delete" => Op::Delete,
                        _ => Op::Insert,
                    };
                    let row: Value = serde_json::from_str(&row_json).unwrap_or(Value::Null);
                    self.last_position = id;
                    yield ChangeEvent { ts, op, table, row };
                }
                if found < 500 {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        })
    }

    fn poll_rows(
        db_path: &PathBuf,
        last_position: i64,
    ) -> Result<Vec<(i64, i64, String, String, String)>> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        // Drain pending trigger-written rows from _cdc_queue into _cdc_log in one batch
        Self::drain_queue(&conn)?;

        let mut stmt = conn.prepare(
            "SELECT id, ts, op, tbl, row_json FROM _cdc_log WHERE id > ?1 ORDER BY id ASC LIMIT 500",
        )?;
        let rows = stmt
            .query_map([last_position], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Move up to BATCH_SIZE rows from _cdc_queue → _cdc_log, then delete them.
    fn drain_queue(conn: &Connection) -> Result<()> {
        conn.execute_batch("BEGIN;")?;
        conn.execute_batch(&format!(
            "INSERT INTO _cdc_log(ts,op,tbl,row_json)
             SELECT ts,op,tbl,row_json FROM _cdc_queue ORDER BY id ASC LIMIT {BATCH_SIZE};
             DELETE FROM _cdc_queue WHERE id IN (
                 SELECT id FROM _cdc_queue ORDER BY id ASC LIMIT {BATCH_SIZE}
             );"
        ))?;
        conn.execute_batch("COMMIT;")?;
        Ok(())
    }

    /// Emit a change event via the batched writer (replaces old emit_direct).
    pub fn emit_direct(db_path: &PathBuf, op: Op, table: &str, row: Value) -> Result<()> {
        // Legacy sync path: write directly to _cdc_log (used by tests / non-async callers).
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        let ts = Utc::now().timestamp_millis();
        let op_str = match op {
            Op::Insert => "insert",
            Op::Update => "update",
            Op::Delete => "delete",
        };
        conn.execute(
            "INSERT INTO _cdc_log(ts,op,tbl,row_json) VALUES(?1,?2,?3,?4)",
            rusqlite::params![ts, op_str, table, serde_json::to_string(&row)?],
        )?;
        Ok(())
    }

    /// Get a handle to the batched writer for async high-throughput paths.
    pub fn writer(&self) -> Arc<CdcWriter> {
        self.writer.clone()
    }
}

/// Background task: drains mpsc queue → SQLite in batches of BATCH_SIZE or FLUSH_MS.
async fn flush_loop(db_path: PathBuf, mut rx: mpsc::Receiver<QueuedEvent>) {
    let mut buf: Vec<QueuedEvent> = Vec::with_capacity(BATCH_SIZE);
    let tick = std::time::Duration::from_millis(FLUSH_MS);

    loop {
        // Collect up to BATCH_SIZE events within FLUSH_MS window
        let deadline = tokio::time::Instant::now() + tick;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(ev)) => {
                    buf.push(ev);
                    if buf.len() >= BATCH_SIZE {
                        break;
                    }
                }
                Ok(None) => return, // channel closed
                Err(_) => break,    // timeout
            }
        }

        if buf.is_empty() {
            continue;
        }

        // Flush synchronously on blocking thread to avoid blocking async runtime
        let events = std::mem::replace(&mut buf, Vec::with_capacity(BATCH_SIZE));
        let path = db_path.clone();
        let result = tokio::task::spawn_blocking(move || flush_batch(&path, &events)).await;
        if let Err(e) = result {
            tracing::error!("CDC flush task panicked: {e}");
        }
    }
}

fn flush_batch(db_path: &PathBuf, events: &[QueuedEvent]) -> Result<()> {
    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

    // Cached prepared statement lives for the duration of this batch
    conn.execute_batch("BEGIN;")?;
    {
        let mut stmt = conn.prepare_cached(
            "INSERT INTO _cdc_log(ts,op,tbl,row_json) VALUES(?1,?2,?3,?4)",
        )?;
        for ev in events {
            stmt.execute(rusqlite::params![ev.ts, ev.op, ev.table, ev.row_json])?;
        }
    }
    conn.execute_batch("COMMIT;")?;
    Ok(())
}
