//! News ingestion: FTS5 headline+body + graph edges to tickers.
//!
//! Schema:
//!   market_news(id INTEGER PRIMARY KEY, ts INTEGER, headline TEXT, body TEXT)
//!   market_news_fts: FTS5 virtual table on headline+body
//!   news_ticker_edges(news_id INTEGER, ticker TEXT)

use crate::error::Result;
use rusqlite::{params, Connection};

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS market_news (
            id       INTEGER PRIMARY KEY AUTOINCREMENT,
            ts       INTEGER NOT NULL,
            headline TEXT NOT NULL,
            body     TEXT NOT NULL DEFAULT ''
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS market_news_fts USING fts5(
            headline, body, content='market_news', content_rowid='id',
            tokenize='unicode61'
        );
        CREATE TABLE IF NOT EXISTS news_ticker_edges (
            news_id INTEGER NOT NULL,
            ticker  TEXT NOT NULL,
            PRIMARY KEY (news_id, ticker)
        );
        CREATE INDEX IF NOT EXISTS idx_news_ts ON market_news(ts);
        CREATE INDEX IF NOT EXISTS idx_edge_ticker ON news_ticker_edges(ticker);
    ",
    )?;
    Ok(())
}

pub fn ingest(
    conn: &Connection,
    ts: i64,
    headline: &str,
    body: &str,
    tickers: &[&str],
) -> Result<i64> {
    conn.execute_batch("BEGIN;")?;
    conn.execute(
        "INSERT INTO market_news (ts, headline, body) VALUES (?1, ?2, ?3)",
        params![ts, headline, body],
    )?;
    let id = conn.last_insert_rowid();
    // Sync FTS5 content table
    conn.execute(
        "INSERT INTO market_news_fts(rowid, headline, body) VALUES (?1, ?2, ?3)",
        params![id, headline, body],
    )?;
    for ticker in tickers {
        conn.execute(
            "INSERT OR IGNORE INTO news_ticker_edges (news_id, ticker) VALUES (?1, ?2)",
            params![id, ticker.to_uppercase()],
        )?;
    }
    conn.execute_batch("COMMIT;")?;
    Ok(id)
}

/// FTS5 search over news; returns (id, ts, headline, score).
pub fn search(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<(i64, i64, String, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.ts, n.headline, bm25(market_news_fts) AS score
         FROM market_news_fts f
         JOIN market_news n ON n.id = f.rowid
         WHERE market_news_fts MATCH ?1
         ORDER BY score
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![query, limit as i64], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get::<_, f64>(3).unwrap_or(0.0),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub use init_schema as NewsStore;
