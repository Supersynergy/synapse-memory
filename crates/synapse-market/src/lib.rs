//! synapse-market — embedded backtest + HFT-sim on Synapse's full stack.
//!
//! Stack:
//!  - OHLCV: SQLite columnar tables `ohlcv_<symbol>` (WAL-batched)
//!  - Regime-vec: per-day feature embeddings → sqlite-vec similarity search
//!  - News-FTS: FTS5 headline+body, graph edges to tickers
//!  - Backtest: deterministic replay, Strategy trait, BacktestReport

pub mod store;
pub mod series;

mod ohlcv;
mod regime;
mod news;
mod backtest;
mod error;

pub use error::{Error, Result};
pub use ohlcv::Ohlcv;
pub use regime::RegimeVec;
pub use news::NewsStore;
pub use backtest::{Strategy, Tick, Order, OrderSide, BacktestReport};

use rusqlite::Connection;
use std::path::Path;

/// Main entry point — wraps a rusqlite Connection + Synapse Store.
pub struct Market {
    pub conn: Connection,
}

impl Market {
    /// Open (or create) a market database at `path`.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-65536;")?;
        news::init_schema(&conn)?;
        Ok(Self { conn })
    }

    /// In-memory ephemeral market (tests / scripts).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        news::init_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Bulk-insert OHLCV rows for `symbol`. Runs inside a single WAL transaction.
    /// Each row: (unix_ts_secs, open, high, low, close, volume).
    pub fn ingest_ohlcv(&self, symbol: &str, rows: &[(i64, f64, f64, f64, f64, f64)]) -> Result<()> {
        ohlcv::ingest(&self.conn, symbol, rows)
    }

    /// Insert a news item and link it to tickers via graph edges.
    /// Returns the rowid of the inserted document.
    pub fn ingest_news(&self, ts: i64, headline: &str, body: &str, tickers: &[&str]) -> Result<i64> {
        news::ingest(&self.conn, ts, headline, body, tickers)
    }

    /// Compute a per-day feature vector for `date_ts` of `symbol` and return
    /// the top-`n` most similar past days by cosine similarity.
    ///
    /// Feature vec: [ret_1d, ret_5d, vol_20d, range_norm, vol_ratio].
    /// Similarity via dot-product over stored f32 blobs (brute-force, <1ms @ 10k days).
    pub fn regime_search(&self, symbol: &str, date_ts: i64, top_n: usize) -> Result<Vec<(i64, f32)>> {
        regime::search(&self.conn, symbol, date_ts, top_n)
    }

    /// Run a full backtest over `symbol` in `[start_ts, end_ts)`.
    /// Calls `strategy.on_tick` for every OHLCV row in order.
    pub fn backtest<S: Strategy>(
        &self,
        symbol: &str,
        start_ts: i64,
        end_ts: i64,
        strategy: &mut S,
    ) -> Result<BacktestReport> {
        backtest::run(&self.conn, symbol, start_ts, end_ts, strategy)
    }
}
