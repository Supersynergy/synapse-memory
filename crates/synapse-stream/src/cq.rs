use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use crate::cdc::ChangeEvent;
use serde_json::Value;
use rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct ContinuousQuery {
    pub sql: String,
    pub window: Duration,
}

pub struct QueryEngine {
    db_path: PathBuf,
    queries: HashMap<String, ContinuousQuery>,
}

impl QueryEngine {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
            queries: HashMap::new(),
        }
    }

    pub fn register_cq(&mut self, name: impl Into<String>, cq: ContinuousQuery) -> Result<()> {
        self.queries.insert(name.into(), cq);
        Ok(())
    }

    /// Run a registered CQ over events within the window.
    /// Returns JSON rows from the in-memory SQLite evaluation.
    pub fn eval(&self, name: &str, events: &[ChangeEvent]) -> Result<Vec<Value>> {
        let cq = self.queries.get(name).ok_or_else(|| anyhow::anyhow!("unknown CQ: {name}"))?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let window_ms = cq.window.as_millis() as i64;
        let cutoff = now_ms - window_ms;

        // Build an in-memory SQLite with the windowed events and run the CQ sql.
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE events (ts INTEGER, op TEXT, tbl TEXT, row_json TEXT);",
        )?;
        {
            let mut ins = conn.prepare(
                "INSERT INTO events(ts,op,tbl,row_json) VALUES(?1,?2,?3,?4)",
            )?;
            for ev in events.iter().filter(|e| e.ts >= cutoff) {
                ins.execute(rusqlite::params![
                    ev.ts,
                    format!("{:?}", ev.op),
                    ev.table,
                    serde_json::to_string(&ev.row)?
                ])?;
            }
        }

        let mut stmt = conn.prepare(&cq.sql)?;
        let col_count = stmt.column_count();
        let col_names: Vec<String> = (0..col_count)
            .map(|i| stmt.column_name(i).unwrap_or("col").to_string())
            .collect();

        let rows = stmt.query_map([], |row| {
            let mut obj = serde_json::Map::new();
            for (i, name) in col_names.iter().enumerate() {
                let val: rusqlite::types::Value = row.get(i)?;
                obj.insert(name.clone(), rusqlite_to_json(val));
            }
            Ok(Value::Object(obj))
        })?
        .filter_map(|r| r.ok())
        .collect();

        Ok(rows)
    }
}

fn rusqlite_to_json(v: rusqlite::types::Value) -> Value {
    match v {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(i) => Value::Number(i.into()),
        rusqlite::types::Value::Real(f) => {
            serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
        rusqlite::types::Value::Text(s) => Value::String(s),
        rusqlite::types::Value::Blob(b) => Value::String(base64_encode(&b)),
    }
}

fn base64_encode(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for byte in b {
        write!(s, "{:02x}", byte).ok();
    }
    s
}
