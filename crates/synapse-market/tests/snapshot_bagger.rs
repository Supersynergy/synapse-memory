/// L3 snapshot/replay test using real bagger.db data (220 tickers × 122 bars).
/// Also logs p50 latency per impl into bench_history.db.
use duckdb::Connection as DuckConn;
use rusqlite::Connection as SqliteConn;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use synapse_market::series::Series;
use synapse_market::store::page::Bar;

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn load_bagger_data() -> HashMap<String, Vec<Bar>> {
    let db_path = home().join(".alphaforge/bagger.db");
    if !db_path.exists() {
        return HashMap::new();
    }
    let conn = SqliteConn::open(&db_path).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT ticker, timestamp, open, high, low, close, volume
         FROM intraday_candles WHERE interval = '15m'
         ORDER BY ticker, timestamp",
        )
        .unwrap();
    let mut map: HashMap<String, Vec<Bar>> = HashMap::new();
    let mut rows = stmt.query([]).unwrap();
    while let Some(r) = rows.next().unwrap() {
        let ticker: String = r.get(0).unwrap();
        let ts: i64 = r.get(1).unwrap();
        let open: f64 = r.get::<_, f64>(2).unwrap_or(0.0);
        let high: f64 = r.get::<_, f64>(3).unwrap_or(0.0);
        let low: f64 = r.get::<_, f64>(4).unwrap_or(0.0);
        let close: f64 = r.get::<_, f64>(5).unwrap_or(0.0);
        let volume: f64 = r.get::<_, f64>(6).unwrap_or(0.0);
        map.entry(ticker).or_default().push(Bar {
            ts,
            open: open as f32,
            high: high as f32,
            low: low as f32,
            close: close as f32,
            volume: volume as f32,
        });
    }
    map
}

fn sqlite_ingest_ticker(conn: &SqliteConn, ticker: &str, bars: &[Bar]) {
    let safe = ticker.replace(['-', '.', '/'], "_");
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS ohlcv_{safe} (
            ts INTEGER PRIMARY KEY, open REAL, high REAL, low REAL, close REAL, volume REAL
         );"
    ))
    .unwrap();
    let sql = format!("INSERT OR IGNORE INTO ohlcv_{safe} VALUES(?,?,?,?,?,?)");
    let mut stmt = conn.prepare_cached(&sql).unwrap();
    for b in bars {
        stmt.execute(rusqlite::params![
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

fn duckdb_ingest_ticker(conn: &DuckConn, ticker: &str, bars: &[Bar]) {
    let safe = ticker.replace(['-', '.', '/'], "_");
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS ohlcv_{safe} (ts BIGINT, open FLOAT, high FLOAT, low FLOAT, close FLOAT, volume FLOAT);"
    )).unwrap();
    let mut app = conn.appender(&format!("ohlcv_{safe}")).unwrap();
    for b in bars {
        app.append_row(duckdb::params![
            b.ts,
            b.open as f64,
            b.high as f64,
            b.low as f64,
            b.close as f64,
            b.volume as f64
        ])
        .unwrap();
    }
    app.flush().unwrap();
}

struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn next_usize(&mut self, hi: usize) -> usize {
        if hi == 0 {
            0
        } else {
            (self.next_u64() % hi as u64) as usize
        }
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn init_bench_history(conn: &SqliteConn) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bench_history (
            run_ts INTEGER,
            impl TEXT,
            p50_us INTEGER,
            n_queries INTEGER
         );",
    )
    .unwrap();
}

#[test]
fn snapshot_bagger_100_queries() {
    let data = load_bagger_data();
    if data.is_empty() {
        eprintln!("SKIP: bagger.db not found — skipping snapshot test");
        return;
    }

    let dir = TempDir::new().unwrap();
    let sq = SqliteConn::open_in_memory().unwrap();
    let dk = DuckConn::open_in_memory().unwrap();

    // Collect tickers + sort for determinism
    let mut tickers: Vec<String> = data.keys().cloned().collect();
    tickers.sort();

    let mut series_map: HashMap<String, Series> = HashMap::new();

    for ticker in &tickers {
        let bars = &data[ticker];
        let safe = ticker.replace(['-', '.', '/'], "_");
        let path = dir.path().join(format!("{safe}.smx"));
        let mut s = Series::open(&path).unwrap();
        s.append(bars).unwrap();
        s.close().unwrap();
        series_map.insert(ticker.clone(), Series::open(&path).unwrap());
        sqlite_ingest_ticker(&sq, ticker, bars);
        duckdb_ingest_ticker(&dk, ticker, bars);
    }

    let mut rng = Lcg::new(0xBAD_CAFE);
    let mut smx_times: Vec<u128> = Vec::with_capacity(100);
    let mut sq_times: Vec<u128> = Vec::with_capacity(100);
    let mut dk_times: Vec<u128> = Vec::with_capacity(100);

    let eps: f32 = 1e-4;

    for _ in 0..100 {
        let ticker = &tickers[rng.next_usize(tickers.len())];
        let bars_all = &data[ticker];
        if bars_all.len() < 10 {
            continue;
        }

        let ts_list: Vec<i64> = bars_all.iter().map(|b| b.ts).collect();
        let ts_min = *ts_list.iter().min().unwrap();
        let ts_max = *ts_list.iter().max().unwrap();
        let range = (ts_max - ts_min).max(1);
        let a = ts_min + (rng.next_f64() * range as f64 * 0.5) as i64;
        let b = a + (rng.next_f64() * range as f64 * 0.5) as i64 + 1;
        let safe = ticker.replace(['-', '.', '/'], "_");

        // Synapse-X
        let t0 = std::time::Instant::now();
        let smx_bars = series_map.get_mut(ticker).unwrap().range(a..b).unwrap();
        smx_times.push(t0.elapsed().as_micros());

        // SQLite
        let t0 = std::time::Instant::now();
        let sq_rows: Vec<(i64, f32, f32, f32, f32, f32)> = {
            let mut stmt = sq.prepare_cached(&format!(
                "SELECT ts,open,high,low,close,volume FROM ohlcv_{safe} WHERE ts>=? AND ts<? ORDER BY ts"
            )).unwrap();
            stmt.query_map(rusqlite::params![a, b], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, f64>(1)? as f32,
                    r.get::<_, f64>(2)? as f32,
                    r.get::<_, f64>(3)? as f32,
                    r.get::<_, f64>(4)? as f32,
                    r.get::<_, f64>(5)? as f32,
                ))
            })
            .unwrap()
            .map(|x| x.unwrap())
            .collect()
        };
        sq_times.push(t0.elapsed().as_micros());

        // DuckDB
        let t0 = std::time::Instant::now();
        let dk_rows: Vec<(i64, f32, f32, f32, f32, f32)> = {
            let mut stmt = dk.prepare(&format!(
                "SELECT ts,open,high,low,close,volume FROM ohlcv_{safe} WHERE ts>=? AND ts<? ORDER BY ts"
            )).unwrap();
            stmt.query_map(duckdb::params![a, b], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, f64>(1)? as f32,
                    r.get::<_, f64>(2)? as f32,
                    r.get::<_, f64>(3)? as f32,
                    r.get::<_, f64>(4)? as f32,
                    r.get::<_, f64>(5)? as f32,
                ))
            })
            .unwrap()
            .map(|x| x.unwrap())
            .collect()
        };
        dk_times.push(t0.elapsed().as_micros());

        // Assert agreement
        assert_eq!(
            smx_bars.len(),
            sq_rows.len(),
            "DIFF ticker={ticker} ts={a}..{b}: smx={} sqlite={}",
            smx_bars.len(),
            sq_rows.len()
        );
        assert_eq!(
            smx_bars.len(),
            dk_rows.len(),
            "DIFF ticker={ticker} ts={a}..{b}: smx={} duckdb={}",
            smx_bars.len(),
            dk_rows.len()
        );

        for (bar, (sq_ts, sq_o, sq_h, sq_l, sq_c, sq_v), (dk_ts, _, _, _, dk_c, _)) in smx_bars
            .iter()
            .zip(sq_rows.iter())
            .zip(dk_rows.iter())
            .map(|((a, b), c)| (a, b, c))
        {
            assert_eq!(bar.ts, *sq_ts);
            assert_eq!(bar.ts, *dk_ts);
            assert!(
                (bar.close - sq_c).abs() <= eps,
                "close diff ticker={ticker} ts={}",
                bar.ts
            );
            assert!(
                (bar.close - dk_c).abs() <= eps,
                "close diff ticker={ticker} ts={}",
                bar.ts
            );
            let _ = (sq_o, sq_h, sq_l, sq_v);
        }
    }

    // Log bench history
    let bench_path = home().join(".synapse-x/bench_history.db");
    if let Some(parent) = bench_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(hist) = SqliteConn::open(&bench_path) {
        init_bench_history(&hist);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let p50 = |mut v: Vec<u128>| {
            v.sort_unstable();
            v[v.len() / 2]
        };
        let smx_p50 = p50(smx_times) as i64;
        let sq_p50 = p50(sq_times) as i64;
        let dk_p50 = p50(dk_times) as i64;

        let _ = hist.execute(
            "INSERT INTO bench_history VALUES(?,?,?,?)",
            rusqlite::params![now, "synapse-x", smx_p50, 100],
        );
        let _ = hist.execute(
            "INSERT INTO bench_history VALUES(?,?,?,?)",
            rusqlite::params![now, "sqlite", sq_p50, 100],
        );
        let _ = hist.execute(
            "INSERT INTO bench_history VALUES(?,?,?,?)",
            rusqlite::params![now, "duckdb", dk_p50, 100],
        );
        eprintln!("bench_history p50(µs): synapse-x={smx_p50} sqlite={sq_p50} duckdb={dk_p50}");
    }
}
