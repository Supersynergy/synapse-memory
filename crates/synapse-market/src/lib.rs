//! synapse-market — embedded backtest + HFT-sim on Synapse's full stack.
//!
//! Stack:
//!  - OHLCV: SQLite columnar tables `ohlcv_<symbol>` (WAL-batched)
//!  - Regime-vec: per-day feature embeddings → sqlite-vec similarity search
//!  - News-FTS: FTS5 headline+body, graph edges to tickers
//!  - Backtest: deterministic replay, Strategy trait, BacktestReport
//!  - JIT filter: Cranelift-compiled predicates (`jit::FilterCache`) —
//!    compile Cmp/And/Or/Not trees to native code, cached by predicate hash.

pub mod pattern;
pub mod filter;
pub mod backtest_wf;
pub mod conformal;
pub mod alert;
pub mod jit;
pub mod learn;
pub mod book;
pub mod store;
pub mod series;
pub mod cache;
pub mod analytics;
pub mod signal;
pub mod router;
pub mod ffi;
pub mod stream;

mod ohlcv;
pub mod regime;
mod news;
mod backtest;
mod error;

pub use error::{Error, Result};
pub use ohlcv::Ohlcv;
pub use regime::RegimeVec;
pub use news::NewsStore;
pub use backtest::{Strategy, Tick, Order, OrderSide, BacktestReport};
pub use signal::turbovec_index::TurboVecIndex;
pub use pattern::{Pattern, Match as PatternMatch, Event as PatternEvent};
pub use pattern::fsm::FsmEngine;
pub use pattern::dsl::parse as parse_pattern;

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

    /// Build a TurboQuant signal-similarity index (recommended default).
    ///
    /// 4-bit quantisation, no training required, ~8× memory vs f32.
    /// Prefer over `signal_index` for new workloads.
    pub fn signal_index_v2(
        &self,
        signals: &[(SignalId, Vec<f32>)],
    ) -> Result<TurboVecIndex> {
        TurboVecIndex::build(signals, 4)
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

    /// Fetch raw OHLCV rows for `symbol` in `[start, end)` unix-seconds range.
    /// Returns `(ts, open, high, low, close, volume)` tuples.
    pub fn fetch_candles(&self, symbol: &str, start: i64, end: i64) -> Result<Vec<(i64, f64, f64, f64, f64, f64)>> {
        ohlcv::fetch_range(&self.conn, symbol, start, end)
    }

    /// Consume a `TickStream` of `LiveTick`s and ingest them as OHLCV rows.
    ///
    /// Ticks are buffered in batches of 1000 and flushed via `ingest_ohlcv`.
    /// Each tick maps: `price → open=high=low=close`, `qty → volume`.
    /// Returns the total number of ticks ingested.
    pub async fn ingest_stream<S>(
        &self,
        ticker: &str,
        mut stream: S,
        max_ticks: Option<usize>,
    ) -> Result<usize>
    where
        S: stream::TickStream<Item = stream::LiveTick>,
    {
        const BATCH: usize = 1000;
        let mut buf: Vec<(i64, f64, f64, f64, f64, f64)> = Vec::with_capacity(BATCH);
        let mut total = 0usize;
        let limit = max_ticks.unwrap_or(usize::MAX);

        while total < limit {
            let Some(tick) = stream.next_tick().await else { break };
            buf.push((tick.ts, tick.price, tick.price, tick.price, tick.price, tick.qty));
            total += 1;
            if buf.len() >= BATCH {
                self.ingest_ohlcv(ticker, &buf)?;
                buf.clear();
            }
        }
        if !buf.is_empty() {
            self.ingest_ohlcv(ticker, &buf)?;
        }
        Ok(total)
    }

    /// Create a fresh FSM engine ready to accept pattern registrations.
    pub fn fsm_engine(&self) -> FsmEngine {
        FsmEngine::new()
    }

    /// Replay stored OHLCV as Candle events through `patterns` and return all matches.
    pub fn scan_patterns(&self, symbol: &str, patterns: &[Pattern]) -> Result<Vec<PatternMatch>> {
        let rows = ohlcv::fetch_range(&self.conn, symbol, i64::MIN, i64::MAX)?;
        let mut engine = FsmEngine::new();
        for p in patterns {
            engine.register(p.clone());
        }
        let mut matches = Vec::new();
        for row in &rows {
            let bar = crate::store::page::Bar {
                ts: row.0,
                open: row.1 as f32,
                high: row.2 as f32,
                low: row.3 as f32,
                close: row.4 as f32,
                volume: row.5 as f32,
            };
            let ev = PatternEvent::Candle(bar);
            matches.extend(engine.on_event(symbol, &ev));
        }
        matches.extend(engine.flush());
        Ok(matches)
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

    /// Walk-forward cross-validation with DSR / PSR / PBO truth-gate.
    pub fn walk_forward<F>(
        &self,
        ts_range: std::ops::Range<i64>,
        folds: u32,
        strategy: F,
    ) -> backtest_wf::WfReport
    where
        F: Fn(&[backtest_wf::Bar]) -> Vec<backtest_wf::TradeResult>,
    {
        backtest_wf::WalkForward::new(folds).run(ts_range, strategy)
    }

    /// Split-conformal prediction from pre-computed non-conformity scores.
    pub fn conformal(scores: &[f32], y: &[f32]) -> conformal::Conformal {
        conformal::Conformal::fit_split(scores, y, |_| 0.0)
    }

    /// Build a fresh AlertEngine.
    pub fn alert_engine() -> alert::AlertEngine {
        alert::AlertEngine::new()
    }
}
