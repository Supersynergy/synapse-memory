use anyhow::Result;
use async_stream::try_stream;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::pin::Pin;
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

pub struct CdcReader {
    pub db_path: PathBuf,
    pub last_position: i64,
}

impl CdcReader {
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self> {
        let db_path = db_path.into();
        let conn = Connection::open(&db_path)?;
        Self::ensure_cdc_table(&conn)?;
        Ok(Self { db_path, last_position: 0 })
    }

    fn ensure_cdc_table(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _cdc_log (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                ts      INTEGER NOT NULL,
                op      TEXT    NOT NULL,
                tbl     TEXT    NOT NULL,
                row_json TEXT   NOT NULL
            );",
        )?;
        Ok(())
    }

    /// Install CDC triggers on `table`. Call once per table you want to watch.
    pub fn install_triggers(&self, table: &str) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        // Collect columns
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
                INSERT INTO _cdc_log(ts,op,tbl,row_json)
                VALUES(unixepoch('now','subsec')*1000,'insert','{t}',json_object({new}));
            END;
            CREATE TRIGGER IF NOT EXISTS _cdc_{t}_update
            AFTER UPDATE ON {t} BEGIN
                INSERT INTO _cdc_log(ts,op,tbl,row_json)
                VALUES(unixepoch('now','subsec')*1000,'update','{t}',json_object({new}));
            END;
            CREATE TRIGGER IF NOT EXISTS _cdc_{t}_delete
            AFTER DELETE ON {t} BEGIN
                INSERT INTO _cdc_log(ts,op,tbl,row_json)
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
                // All rusqlite work is sync and completed before any await.
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

    fn poll_rows(db_path: &PathBuf, last_position: i64)
        -> Result<Vec<(i64, i64, String, String, String)>>
    {
        let conn = Connection::open(db_path)?;
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

    /// Emit a change event directly (used by tests / non-trigger paths).
    pub fn emit_direct(db_path: &PathBuf, op: Op, table: &str, row: Value) -> Result<()> {
        let conn = Connection::open(db_path)?;
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
}
