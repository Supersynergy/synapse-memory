//! OHLCV columnar storage: one SQLite table per symbol.
//! Schema: `ohlcv_<symbol>(ts INTEGER PRIMARY KEY, open REAL, high REAL, low REAL, close REAL, volume REAL)`
//! Bulk-insert via WAL transaction → ≥1M ticks/sec on NVMe/M4.

use crate::error::Result;
use rusqlite::{params, Connection};

fn table_name(symbol: &str) -> String {
    // Sanitize: allow alphanumeric + underscore only.
    let safe: String = symbol
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("ohlcv_{}", safe.to_uppercase())
}

pub fn ensure_table(conn: &Connection, symbol: &str) -> Result<()> {
    let t = table_name(symbol);
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {t} (
            ts     INTEGER PRIMARY KEY,
            open   REAL NOT NULL,
            high   REAL NOT NULL,
            low    REAL NOT NULL,
            close  REAL NOT NULL,
            volume REAL NOT NULL
        ) WITHOUT ROWID;"
    ))?;
    Ok(())
}

pub fn ingest(
    conn: &Connection,
    symbol: &str,
    rows: &[(i64, f64, f64, f64, f64, f64)],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    ensure_table(conn, symbol)?;
    let t = table_name(symbol);
    let sql = format!(
        "INSERT OR REPLACE INTO {t} (ts, open, high, low, close, volume) VALUES (?1,?2,?3,?4,?5,?6)"
    );
    // Single transaction — WAL allows readers concurrently.
    conn.execute_batch("BEGIN;")?;
    {
        let mut stmt = conn.prepare_cached(&sql)?;
        for &(ts, o, h, l, c, v) in rows {
            stmt.execute(params![ts, o, h, l, c, v])?;
        }
    }
    conn.execute_batch("COMMIT;")?;
    Ok(())
}

/// Fetch rows in [start, end) ordered by ts.
pub fn fetch_range(
    conn: &Connection,
    symbol: &str,
    start: i64,
    end: i64,
) -> Result<Vec<(i64, f64, f64, f64, f64, f64)>> {
    let t = table_name(symbol);
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT ts, open, high, low, close, volume FROM {t} WHERE ts >= ?1 AND ts < ?2 ORDER BY ts"
    ))?;
    let rows = stmt
        .query_map(params![start, end], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Fetch all rows ordered by ts.
pub fn fetch_all(conn: &Connection, symbol: &str) -> Result<Vec<(i64, f64, f64, f64, f64, f64)>> {
    let t = table_name(symbol);
    // table may not exist yet
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            params![t],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !exists {
        return Ok(vec![]);
    }
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT ts, open, high, low, close, volume FROM {t} ORDER BY ts"
    ))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub use self::ensure_table as Ohlcv;
