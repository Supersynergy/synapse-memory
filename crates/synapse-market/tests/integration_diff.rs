/// L3 Differential test: Synapse-X vs SQLite vs DuckDB on 1000 random queries.
/// All three must agree to ≤EPS on every result cell.
use duckdb::Connection as DuckConn;
use rusqlite::Connection as SqliteConn;
use std::ops::Range;
use tempfile::TempDir;

use synapse_market::series::Series;
use synapse_market::store::page::Bar;

// ── Deterministic PRNG (LCG) ─────────────────────────────────────────────────
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
    fn next_f64(&mut self) -> f64 { (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 }
    fn next_usize(&mut self, hi: usize) -> usize { (self.next_u64() % hi as u64) as usize }
}

// ── Fixture ───────────────────────────────────────────────────────────────────
const N_TICKERS: usize = 50;
const N_DAYS: usize = 5 * 252;
const BASE_TS: i64 = 1_600_000_000i64;
const DAY_SECS: i64 = 86_400;

fn make_bars(ticker_idx: usize) -> Vec<Bar> {
    let mut rng = Lcg::new(0xDEAD_BEEF ^ ticker_idx as u64);
    let mut close = 100.0f32 + ticker_idx as f32;
    (0..N_DAYS)
        .map(|day| {
            let ret = (rng.next_f64() as f32 - 0.5) * 0.04;
            let open = close;
            close = (open * (1.0 + ret)).max(1.0);
            let high = open.max(close) * (1.0 + rng.next_f64() as f32 * 0.01);
            let low  = open.min(close) * (1.0 - rng.next_f64() as f32 * 0.01);
            let vol  = 1_000_000.0f32 * (0.5 + rng.next_f64() as f32);
            Bar {
                ts: BASE_TS + day as i64 * DAY_SECS,
                open, high, low, close, volume: vol,
            }
        })
        .collect()
}

// ── SQLite helpers ────────────────────────────────────────────────────────────
fn sqlite_ingest(conn: &SqliteConn, ticker: &str, bars: &[Bar]) {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS ohlcv_{ticker} (
            ts INTEGER PRIMARY KEY, open REAL, high REAL, low REAL, close REAL, volume REAL
         ) WITHOUT ROWID;"
    )).unwrap();
    let mut stmt = conn.prepare_cached(&format!(
        "INSERT OR IGNORE INTO ohlcv_{ticker} VALUES(?,?,?,?,?,?)"
    )).unwrap();
    for b in bars {
        stmt.execute(rusqlite::params![
            b.ts, b.open as f64, b.high as f64, b.low as f64, b.close as f64, b.volume as f64
        ]).unwrap();
    }
}

fn sqlite_range(conn: &SqliteConn, ticker: &str, ts_start: i64, ts_end: i64) -> Vec<(i64, f32, f32, f32, f32, f32)> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT ts,open,high,low,close,volume FROM ohlcv_{ticker} WHERE ts>=? AND ts<? ORDER BY ts"
    )).unwrap();
    stmt.query_map(rusqlite::params![ts_start, ts_end], |r| Ok((
        r.get::<_,i64>(0)?,
        r.get::<_,f64>(1)? as f32, r.get::<_,f64>(2)? as f32,
        r.get::<_,f64>(3)? as f32, r.get::<_,f64>(4)? as f32,
        r.get::<_,f64>(5)? as f32,
    ))).unwrap().map(|r| r.unwrap()).collect()
}

fn sqlite_range_price(conn: &SqliteConn, ticker: &str, ts_start: i64, ts_end: i64, p_lo: f32, p_hi: f32) -> Vec<(i64, f32)> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT ts,close FROM ohlcv_{ticker} WHERE ts>=? AND ts<? AND close>=? AND close<? ORDER BY ts"
    )).unwrap();
    stmt.query_map(rusqlite::params![ts_start, ts_end, p_lo as f64, p_hi as f64], |r| {
        Ok((r.get::<_,i64>(0)?, r.get::<_,f64>(1)? as f32))
    }).unwrap().map(|r| r.unwrap()).collect()
}

fn sqlite_agg(conn: &SqliteConn, ticker: &str, ts_start: i64, ts_end: i64) -> (f64, f64, f64) {
    conn.query_row(
        &format!("SELECT avg(close),min(close),max(close) FROM ohlcv_{ticker} WHERE ts>=? AND ts<?"),
        rusqlite::params![ts_start, ts_end],
        |r| Ok((r.get::<_,f64>(0).unwrap_or(0.0), r.get::<_,f64>(1).unwrap_or(0.0), r.get::<_,f64>(2).unwrap_or(0.0)))
    ).unwrap()
}

// ── DuckDB helpers ────────────────────────────────────────────────────────────
fn duckdb_ingest(conn: &DuckConn, ticker: &str, bars: &[Bar]) {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS ohlcv_{ticker} (
            ts BIGINT, open FLOAT, high FLOAT, low FLOAT, close FLOAT, volume FLOAT
         );"
    )).unwrap();
    let mut app = conn.appender(&format!("ohlcv_{ticker}")).unwrap();
    for b in bars {
        app.append_row(duckdb::params![
            b.ts, b.open as f64, b.high as f64, b.low as f64, b.close as f64, b.volume as f64
        ]).unwrap();
    }
    app.flush().unwrap();
}

fn duckdb_range(conn: &DuckConn, ticker: &str, ts_start: i64, ts_end: i64) -> Vec<(i64, f32, f32, f32, f32, f32)> {
    let mut stmt = conn.prepare(&format!(
        "SELECT ts,open,high,low,close,volume FROM ohlcv_{ticker} WHERE ts>=? AND ts<? ORDER BY ts"
    )).unwrap();
    let rows = stmt.query_map(duckdb::params![ts_start, ts_end], |r| Ok((
        r.get::<_,i64>(0)?,
        r.get::<_,f64>(1)? as f32, r.get::<_,f64>(2)? as f32,
        r.get::<_,f64>(3)? as f32, r.get::<_,f64>(4)? as f32,
        r.get::<_,f64>(5)? as f32,
    ))).unwrap();
    rows.map(|r| r.unwrap()).collect()
}

fn duckdb_range_price(conn: &DuckConn, ticker: &str, ts_start: i64, ts_end: i64, p_lo: f32, p_hi: f32) -> Vec<(i64, f32)> {
    let mut stmt = conn.prepare(&format!(
        "SELECT ts,close FROM ohlcv_{ticker} WHERE ts>=? AND ts<? AND close>=? AND close<? ORDER BY ts"
    )).unwrap();
    let rows = stmt.query_map(duckdb::params![ts_start, ts_end, p_lo as f64, p_hi as f64], |r| {
        Ok((r.get::<_,i64>(0)?, r.get::<_,f64>(1)? as f32))
    }).unwrap();
    rows.map(|r| r.unwrap()).collect()
}

fn duckdb_agg(conn: &DuckConn, ticker: &str, ts_start: i64, ts_end: i64) -> (f64, f64, f64) {
    let mut stmt = conn.prepare(&format!(
        "SELECT avg(close),min(close),max(close) FROM ohlcv_{ticker} WHERE ts>=? AND ts<?"
    )).unwrap();
    stmt.query_row(duckdb::params![ts_start, ts_end], |r| {
        Ok((r.get::<_,f64>(0).unwrap_or(0.0), r.get::<_,f64>(1).unwrap_or(0.0), r.get::<_,f64>(2).unwrap_or(0.0)))
    }).unwrap()
}

// ── Synapse-X helpers ─────────────────────────────────────────────────────────
fn smx_range(series: &mut Series, ts_range: Range<i64>) -> Vec<Bar> {
    series.range(ts_range).unwrap()
}

fn smx_range_price(series: &mut Series, ts_range: Range<i64>, p_lo: f32, p_hi: f32) -> Vec<Bar> {
    series.range(ts_range).unwrap()
        .into_iter()
        .filter(|b| b.close >= p_lo && b.close < p_hi)
        .collect()
}

fn smx_agg(bars: &[Bar]) -> (f64, f64, f64) {
    if bars.is_empty() { return (0.0, 0.0, 0.0); }
    let sum: f64 = bars.iter().map(|b| b.close as f64).sum();
    let avg = sum / bars.len() as f64;
    let min = bars.iter().map(|b| b.close as f64).fold(f64::MAX, f64::min);
    let max = bars.iter().map(|b| b.close as f64).fold(f64::MIN, f64::max);
    (avg, min, max)
}

// ── Assert helpers ────────────────────────────────────────────────────────────
const EPS: f32 = 1e-4;

fn assert_rows_match(
    smx: &[Bar],
    sq: &[(i64, f32, f32, f32, f32, f32)],
    dk: &[(i64, f32, f32, f32, f32, f32)],
    ticker: &str, ts_start: i64,
) {
    assert_eq!(smx.len(), sq.len(),
        "DIFF len ticker={ticker} ts={ts_start}: smx={} sqlite={}", smx.len(), sq.len());
    assert_eq!(smx.len(), dk.len(),
        "DIFF len ticker={ticker} ts={ts_start}: smx={} duckdb={}", smx.len(), dk.len());
    for (bar, (sq_ts, sq_o, sq_h, sq_l, sq_c, sq_v), (dk_ts, dk_o, dk_h, dk_l, dk_c, dk_v)) in
        smx.iter()
           .zip(sq.iter())
           .zip(dk.iter())
           .map(|((a, b), c)| (a, b, c))
    {
        assert_eq!(bar.ts, *sq_ts, "DIFF ts ticker={ticker}");
        assert_eq!(bar.ts, *dk_ts, "DIFF ts ticker={ticker} (duckdb)");
        for (col, bv, sv, dv) in [
            ("open",   bar.open,   *sq_o, *dk_o),
            ("high",   bar.high,   *sq_h, *dk_h),
            ("low",    bar.low,    *sq_l, *dk_l),
            ("close",  bar.close,  *sq_c, *dk_c),
            ("volume", bar.volume, *sq_v, *dk_v),
        ] {
            assert!((bv - sv).abs() <= EPS,
                "DIFF ticker={ticker} ts={} col={col}: smx={bv} sqlite={sv} delta={}", bar.ts, (bv-sv).abs());
            assert!((bv - dv).abs() <= EPS,
                "DIFF ticker={ticker} ts={} col={col}: smx={bv} duckdb={dv} delta={}", bar.ts, (bv-dv).abs());
        }
    }
}

fn assert_price_match(smx: &[Bar], sq: &[(i64, f32)], dk: &[(i64, f32)], ticker: &str, ts_start: i64) {
    assert_eq!(smx.len(), sq.len(),
        "DIFF price ticker={ticker} ts={ts_start}: smx={} sqlite={}", smx.len(), sq.len());
    assert_eq!(smx.len(), dk.len(),
        "DIFF price ticker={ticker} ts={ts_start}: smx={} duckdb={}", smx.len(), dk.len());
    for ((b, sq_row), dk_row) in smx.iter().zip(sq.iter()).zip(dk.iter()) {
        let (sq_ts, sq_c) = sq_row;
        let (dk_ts, dk_c) = dk_row;
        assert_eq!(b.ts, *sq_ts);
        assert_eq!(b.ts, *dk_ts);
        assert!((b.close - sq_c).abs() <= EPS, "close DIFF ticker={ticker} ts={}", b.ts);
        assert!((b.close - dk_c).abs() <= EPS, "close DIFF ticker={ticker} ts={}", b.ts);
    }
}

fn assert_agg_match(smx: (f64, f64, f64), sq: (f64, f64, f64), dk: (f64, f64, f64), ticker: &str, ts_start: i64) {
    let eps64 = 1e-2f64;
    for (label, mv, sv, dv) in [
        ("avg", smx.0, sq.0, dk.0),
        ("min", smx.1, sq.1, dk.1),
        ("max", smx.2, sq.2, dk.2),
    ] {
        assert!((mv - sv).abs() <= eps64,
            "DIFF agg ticker={ticker} ts={ts_start} {label}: smx={mv} sqlite={sv}");
        assert!((mv - dv).abs() <= eps64,
            "DIFF agg ticker={ticker} ts={ts_start} {label}: smx={mv} duckdb={dv}");
    }
}

// ── Main test ─────────────────────────────────────────────────────────────────
#[test]
fn l3_differential_1000_queries() {
    let dir = TempDir::new().unwrap();
    let sq = SqliteConn::open_in_memory().unwrap();
    let dk = DuckConn::open_in_memory().unwrap();

    let tickers: Vec<String> = (0..N_TICKERS).map(|i| format!("T{i:03}")).collect();

    let mut series_vec: Vec<Series> = Vec::with_capacity(N_TICKERS);
    for (idx, ticker) in tickers.iter().enumerate() {
        let bars = make_bars(idx);
        let path = dir.path().join(format!("{ticker}.smx"));
        let mut s = Series::open(&path).unwrap();
        s.append(&bars).unwrap();
        s.close().unwrap();
        series_vec.push(Series::open(&path).unwrap());
        sqlite_ingest(&sq, ticker, &bars);
        duckdb_ingest(&dk, ticker, &bars);
    }

    let mut rng = Lcg::new(0xABCD_1234);
    let total_ts = N_DAYS as i64 * DAY_SECS;

    for q in 0..1000usize {
        let ticker_idx = rng.next_usize(N_TICKERS);
        let ticker = &tickers[ticker_idx];
        let series = &mut series_vec[ticker_idx];

        let a = BASE_TS + rng.next_usize(N_DAYS - 30) as i64 * DAY_SECS;
        let window = (30 + rng.next_usize(N_DAYS / 4)) as i64 * DAY_SECS;
        let b = (a + window).min(BASE_TS + total_ts);

        match q % 4 {
            0 => {
                let smx = smx_range(series, a..b);
                let sq_r = sqlite_range(&sq, ticker, a, b);
                let dk_r = duckdb_range(&dk, ticker, a, b);
                assert_rows_match(&smx, &sq_r, &dk_r, ticker, a);
            }
            1 => {
                let mid_a = a + window / 4;
                let mid_b = mid_a + window / 2;
                let smx = smx_range(series, mid_a..mid_b);
                let sq_r = sqlite_range(&sq, ticker, mid_a, mid_b);
                let dk_r = duckdb_range(&dk, ticker, mid_a, mid_b);
                assert_rows_match(&smx, &sq_r, &dk_r, ticker, mid_a);
            }
            2 => {
                let full = smx_range(series, a..b);
                if full.is_empty() { continue; }
                let prices: Vec<f32> = full.iter().map(|x| x.close).collect();
                let p_lo = prices.iter().cloned().fold(f32::MAX, f32::min) * 0.95;
                let p_hi = prices.iter().cloned().fold(f32::MIN, f32::max) * 1.05;
                let smx_pf = smx_range_price(series, a..b, p_lo, p_hi);
                let sq_pf  = sqlite_range_price(&sq, ticker, a, b, p_lo, p_hi);
                let dk_pf  = duckdb_range_price(&dk, ticker, a, b, p_lo, p_hi);
                assert_price_match(&smx_pf, &sq_pf, &dk_pf, ticker, a);
            }
            _ => {
                let bars = smx_range(series, a..b);
                if bars.is_empty() { continue; }
                let smx_agg = smx_agg(&bars);
                let sq_agg  = sqlite_agg(&sq, ticker, a, b);
                let dk_agg  = duckdb_agg(&dk, ticker, a, b);
                assert_agg_match(smx_agg, sq_agg, dk_agg, ticker, a);
            }
        }
    }
}
