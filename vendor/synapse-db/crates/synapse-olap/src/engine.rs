use anyhow::{Context, Result};
use duckdb::arrow::array::RecordBatch;
use duckdb::{Arrow, Connection};
use std::path::Path;

/// Embedded DuckDB OLAP engine with optional SQLite ATTACH.
pub struct OlapEngine {
    conn: Connection,
}

impl OlapEngine {
    /// Open an in-memory DuckDB instance.
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("duckdb open_in_memory")?;
        Ok(Self { conn })
    }

    /// Open a persistent DuckDB database file.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("duckdb open")?;
        Ok(Self { conn })
    }

    /// Attach an existing SQLite database read-only.
    /// After attach, tables are accessible as `<alias>.<table>`.
    pub fn attach_synapse_db(&mut self, path: &Path, alias: &str) -> Result<()> {
        let p = path.to_string_lossy();
        self.conn
            .execute_batch(&format!(
                "INSTALL sqlite; LOAD sqlite; \
                 ATTACH '{p}' AS {alias} (TYPE sqlite, READ_ONLY);"
            ))
            .context("attach sqlite db")?;
        Ok(())
    }

    /// Execute an OLAP SQL query and return Arrow RecordBatches.
    pub fn query(&self, sql: &str) -> Result<Vec<RecordBatch>> {
        let mut stmt = self.conn.prepare(sql).context("duckdb prepare")?;
        let batches: Vec<RecordBatch> = stmt
            .query_arrow([])
            .context("duckdb query_arrow")?
            .collect();
        Ok(batches)
    }

    /// Execute a statement that returns no rows (DDL, INSERT, etc.).
    pub fn execute(&self, sql: &str) -> Result<()> {
        self.conn.execute_batch(sql).context("duckdb execute_batch")
    }
}
