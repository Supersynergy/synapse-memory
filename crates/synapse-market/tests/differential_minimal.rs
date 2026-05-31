use duckdb::Connection as DuckConn;
use rusqlite::Connection;
use std::collections::HashMap;
use synapse_market::series::Series;
use synapse_market::store::page::Bar;
use tempfile::TempDir;

const TICKERS: usize = 50;
const BARS_PER_TICKER: usize = 252;
const QUERIES: usize = 100;

// Simple LCG RNG (deterministic, no external dep)
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn next_usize(&mut self, n: usize) -> usize {
        (self.next_u64() as usize) % n
    }
    fn next_i64(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() as i64).abs() % (hi - lo)
    }
}

fn gen_bars(rng: &mut Rng, base_ts: i64) -> Vec<Bar> {
    let mut price = 100.0 + rng.next_f64() * 50.0;
    (0..BARS_PER_TICKER)
        .map(|i| {
            let open = price as f32;
            let high = (price + rng.next_f64() * 2.0) as f32;
            let low = (price - rng.next_f64() * 2.0) as f32;
            let close = (price + (rng.next_f64() - 0.5) * 1.5) as f32;
            let volume = (1000.0 + rng.next_f64() * 9000.0) as f32;
            price = close as f64;
            Bar {
                ts: base_ts + i as i64 * 86400,
                open,
                high,
                low,
                close,
                volume,
            }
        })
        .collect()
}

#[test]
fn differential_minimal() {
    let tmpdir = TempDir::new().unwrap();
    let mut rng = Rng::new(0xdeadbeef_cafebabe);

    let tickers: Vec<String> = (0..TICKERS).map(|i| format!("TK{:03}", i)).collect();
    let base_ts: i64 = 1_700_000_000;

    // ── Generate data ──────────────────────────────────────────────────────────
    let mut all_bars: HashMap<String, Vec<Bar>> = HashMap::new();
    for t in &tickers {
        let bars = gen_bars(&mut rng, base_ts);
        all_bars.insert(t.clone(), bars);
    }

    // ── Ingest into Synapse-X (SMX) ────────────────────────────────────────────
    for t in &tickers {
        let path = tmpdir.path().join(format!("{}.smx", t));
        let mut s = Series::open(&path).unwrap();
        s.append(all_bars[t].as_slice()).unwrap();
        s.close().unwrap();
    }

    // ── Ingest into SQLite ─────────────────────────────────────────────────────
    let sql_conn = Connection::open_in_memory().unwrap();
    sql_conn.execute_batch(
        "CREATE TABLE candles (ticker TEXT, ts INTEGER, open REAL, high REAL, low REAL, close REAL, volume REAL,
         PRIMARY KEY (ticker, ts)) WITHOUT ROWID;"
    ).unwrap();
    {
        let mut stmt = sql_conn
            .prepare("INSERT INTO candles VALUES (?,?,?,?,?,?,?)")
            .unwrap();
        for t in &tickers {
            for b in &all_bars[t] {
                stmt.execute(rusqlite::params![
                    t,
                    b.ts,
                    b.open as f64,
                    b.high as f64,
                    b.low as f64,
                    b.close as f64,
                    b.volume as f64
                ])
                .unwrap();
            }
        }
    }

    // ── Ingest into DuckDB ─────────────────────────────────────────────────────
    let ddb = DuckConn::open_in_memory().unwrap();
    ddb.execute_batch(
        "CREATE TABLE candles (ticker VARCHAR, ts BIGINT, open DOUBLE, high DOUBLE, low DOUBLE, close DOUBLE, volume DOUBLE);"
    ).unwrap();
    {
        let mut app = ddb.appender("candles").unwrap();
        for t in &tickers {
            for b in &all_bars[t] {
                app.append_row(duckdb::params![
                    t.as_str(),
                    b.ts,
                    b.open as f64,
                    b.high as f64,
                    b.low as f64,
                    b.close as f64,
                    b.volume as f64
                ])
                .unwrap();
            }
        }
        app.flush().unwrap();
    }

    // ── Queries ────────────────────────────────────────────────────────────────
    let ts_end = base_ts + BARS_PER_TICKER as i64 * 86400;
    let mut total_assertions = 0usize;
    let mut disagreements = 0usize;

    for _ in 0..QUERIES {
        let ticker = &tickers[rng.next_usize(TICKERS)];
        let ts_a = rng.next_i64(base_ts, ts_end - 1);
        let ts_b = rng.next_i64(ts_a + 86400, ts_end);
        let ts_range = ts_a..ts_b;

        // SMX
        let smx_path = tmpdir.path().join(format!("{}.smx", ticker));
        let mut s = Series::open(&smx_path).unwrap();
        let smx_bars = s.range(ts_range.clone()).unwrap();
        let smx: Vec<(i64, f32)> = smx_bars.iter().map(|b| (b.ts, b.close)).collect();

        // SQLite
        let mut sql_stmt = sql_conn
            .prepare_cached(
                "SELECT ts, close FROM candles WHERE ticker=? AND ts>=? AND ts<? ORDER BY ts",
            )
            .unwrap();
        let sql: Vec<(i64, f64)> = sql_stmt
            .query_map(rusqlite::params![ticker, ts_a, ts_b], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        // DuckDB
        let mut ddb_stmt = ddb
            .prepare("SELECT ts, close FROM candles WHERE ticker=? AND ts>=? AND ts<? ORDER BY ts")
            .unwrap();
        let ddb_rows: Vec<(i64, f64)> = ddb_stmt
            .query_map(duckdb::params![ticker.as_str(), ts_a, ts_b], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        // Length check
        if smx.len() != sql.len() || smx.len() != ddb_rows.len() {
            eprintln!(
                "LENGTH MISMATCH ticker={} ts=[{},{}) smx={} sql={} ddb={}",
                ticker,
                ts_a,
                ts_b,
                smx.len(),
                sql.len(),
                ddb_rows.len()
            );
            disagreements += 1;
            continue;
        }

        // Value checks
        for i in 0..smx.len() {
            let (smx_ts, smx_close) = smx[i];
            let (sql_ts, sql_close) = sql[i];
            let (ddb_ts, ddb_close) = ddb_rows[i];

            assert_eq!(smx_ts, sql_ts, "ts mismatch smx vs sql");
            assert_eq!(smx_ts, ddb_ts, "ts mismatch smx vs ddb");

            let diff_sql = (smx_close as f64 - sql_close).abs();
            let diff_ddb = (smx_close as f64 - ddb_close).abs();
            let max_diff = diff_sql.max(diff_ddb);

            if max_diff > 1e-6 {
                eprintln!(
                    "VALUE DIFF ticker={} ts={} smx_close={} sql_close={} ddb_close={} max_diff={}",
                    ticker, smx_ts, smx_close, sql_close, ddb_close, max_diff
                );
                disagreements += 1;
            }
            total_assertions += 1;
        }
    }

    assert_eq!(
        disagreements, 0,
        "{} 3-way disagreements found",
        disagreements
    );
    eprintln!(
        "differential_minimal: {} queries, {} assertions passed",
        QUERIES, total_assertions
    );
}

#[test]
fn aggregate_avg_close() {
    let tmpdir = TempDir::new().unwrap();
    let mut rng = Rng::new(0x1234567890abcdef);

    let tickers: Vec<String> = (0..TICKERS).map(|i| format!("AG{:03}", i)).collect();
    let base_ts: i64 = 1_700_000_000;

    let mut all_bars: HashMap<String, Vec<Bar>> = HashMap::new();
    for t in &tickers {
        all_bars.insert(t.clone(), gen_bars(&mut rng, base_ts));
    }

    // SMX ingest
    for t in &tickers {
        let path = tmpdir.path().join(format!("{}.smx", t));
        let mut s = Series::open(&path).unwrap();
        s.append(all_bars[t].as_slice()).unwrap();
        s.close().unwrap();
    }

    // SQLite ingest
    let sql_conn = Connection::open_in_memory().unwrap();
    sql_conn.execute_batch(
        "CREATE TABLE candles (ticker TEXT, ts INTEGER, close REAL, PRIMARY KEY (ticker, ts)) WITHOUT ROWID;"
    ).unwrap();
    {
        let mut stmt = sql_conn
            .prepare("INSERT INTO candles VALUES (?,?,?)")
            .unwrap();
        for t in &tickers {
            for b in &all_bars[t] {
                stmt.execute(rusqlite::params![t, b.ts, b.close as f64])
                    .unwrap();
            }
        }
    }

    // DuckDB ingest
    let ddb = DuckConn::open_in_memory().unwrap();
    ddb.execute_batch("CREATE TABLE candles (ticker VARCHAR, ts BIGINT, close DOUBLE);")
        .unwrap();
    {
        let mut app = ddb.appender("candles").unwrap();
        for t in &tickers {
            for b in &all_bars[t] {
                app.append_row(duckdb::params![t.as_str(), b.ts, b.close as f64])
                    .unwrap();
            }
        }
        app.flush().unwrap();
    }

    let ts_end = base_ts + BARS_PER_TICKER as i64 * 86400;
    let ts_range = base_ts..ts_end;

    let mut disagreements = 0usize;
    for t in &tickers {
        // SMX avg
        let smx_path = tmpdir.path().join(format!("{}.smx", t));
        let mut s = Series::open(&smx_path).unwrap();
        let bars = s.range(ts_range.clone()).unwrap();
        let smx_avg = if bars.is_empty() {
            0.0
        } else {
            bars.iter().map(|b| b.close as f64).sum::<f64>() / bars.len() as f64
        };

        // SQLite avg
        let sql_avg: f64 = sql_conn
            .query_row(
                "SELECT AVG(close) FROM candles WHERE ticker=? AND ts>=? AND ts<?",
                rusqlite::params![t, base_ts, ts_end],
                |row| row.get(0),
            )
            .unwrap();

        // DuckDB avg
        let mut stmt = ddb
            .prepare("SELECT AVG(close) FROM candles WHERE ticker=? AND ts>=? AND ts<?")
            .unwrap();
        let ddb_avg: f64 = stmt
            .query_row(duckdb::params![t.as_str(), base_ts, ts_end], |row| {
                row.get(0)
            })
            .unwrap();

        let diff1 = (smx_avg - sql_avg).abs();
        let diff2 = (smx_avg - ddb_avg).abs();
        if diff1 > 1e-6 || diff2 > 1e-6 {
            eprintln!(
                "AVG DIFF ticker={} smx={} sql={} ddb={} diff_sql={} diff_ddb={}",
                t, smx_avg, sql_avg, ddb_avg, diff1, diff2
            );
            disagreements += 1;
        }
    }
    assert_eq!(
        disagreements, 0,
        "{} aggregate disagreements",
        disagreements
    );
    eprintln!("aggregate_avg_close: {} tickers all agree", TICKERS);
}
