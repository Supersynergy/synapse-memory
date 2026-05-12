//! synapse-market — embedded backtest + HFT-sim on Synapse's full stack.
//!
//! Stack:
//!  - OHLCV: SQLite columnar tables `ohlcv_<symbol>` (WAL-batched)
//!  - Regime-vec: per-day feature embeddings → sqlite-vec similarity search
//!  - News-FTS: FTS5 headline+body, graph edges to tickers
//!  - Backtest: deterministic replay, Strategy trait, BacktestReport

pub mod filter;
// pub mod jit;  // blocked: cranelift 0.131 BlockArg API churn (see JIT_BLOCKERS.md)
pub mod book;
pub mod store;
pub mod series;
pub mod cache;
pub mod analytics;
pub mod signal;
pub mod router;
pub mod ffi;

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
use std::ops::Range;
use signal::similar::RabitqSignalIndex;
use signal::SignalId;
use analytics::{correlation_matrix_amx, CorrMatrix};

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

    /// Build a RaBitQ signal-similarity index from (id, vec) pairs.
    /// `n_clusters` ~ sqrt(N). Returns shared index; persist with `index.save(path)`.
    pub fn signal_index(
        &self,
        signals: &[(SignalId, Vec<f32>)],
        n_clusters: usize,
    ) -> Result<RabitqSignalIndex> {
        RabitqSignalIndex::build(signals, n_clusters)
    }

    /// Compute the Pearson correlation matrix for `tickers` over `ts_range`.
    ///
    /// Each ticker's close prices in the range are fetched, aligned to the
    /// shortest series length, assembled into a row-major f32 matrix, then
    /// dispatched to the AMX/Accelerate kernel on macOS aarch64 (≥20× NEON).
    /// Returns a [`CorrMatrix`] (n×n, row-major).
    pub fn correlation_matrix(&self, tickers: &[&str], ts_range: Range<i64>) -> Result<CorrMatrix> {
        let mut series: Vec<Vec<f32>> = Vec::with_capacity(tickers.len());
        for t in tickers {
            let rows = ohlcv::fetch_range(&self.conn, t, ts_range.start, ts_range.end)?;
            let closes: Vec<f32> = rows.iter().map(|r| r.4 as f32).collect();
            series.push(closes);
        }
        let cols = series.len();
        let rows = series.iter().map(|s| s.len()).min().unwrap_or(0);
        // Build row-major matrix: rows observations × cols variables
        let mut mat = vec![0.0f32; rows * cols];
        for (c, s) in series.iter().enumerate() {
            for r in 0..rows {
                mat[r * cols + c] = s[r];
            }
        }
        let data = correlation_matrix_amx(&mat, rows, cols);
        Ok(CorrMatrix { data, n: cols })
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
