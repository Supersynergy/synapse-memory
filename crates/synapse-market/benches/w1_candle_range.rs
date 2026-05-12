/// W1 bench: candle-range scan, Synapse-X mmap-pages vs SQLite WITHOUT ROWID.
/// Workload: 60d of 15m candles (~2880 bars), 100 tickers, 1000 random range-queries.
use std::time::{Duration, Instant};
use std::fs;
use tempfile::TempDir;
use rusqlite::Connection;

use synapse_market::store::page::Bar;
use synapse_market::series::Series;

const TICKERS: usize = 100;
const BARS_PER_TICKER: usize = 2880; // 60d × 24h × 4 per hour (15m)
const ITERS: usize = 1000;
const BASE_TS: i64 = 1_700_000_000;

fn make_bars(ticker_id: usize) -> Vec<Bar> {
    let price_base = 10.0 + ticker_id as f32 * 0.5;
    (0..BARS_PER_TICKER)
        .map(|i| Bar {
            ts: BASE_TS + i as i64 * 900,
            open: price_base + (i % 13) as f32 * 0.01,
            high: price_base + (i % 17) as f32 * 0.02,
            low: price_base - (i % 11) as f32 * 0.01,
            close: price_base + (i % 7) as f32 * 0.015,
            volume: 10_000.0 + (i % 100) as f32 * 50.0,
        })
        .collect()
}

// ── SQLite setup ─────────────────────────────────────────────────────────────

fn setup_sqlite(dir: &TempDir) -> Connection {
    let path = dir.path().join("bench.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA cache_size=-65536;
         CREATE TABLE IF NOT EXISTS candles (
             ticker INTEGER NOT NULL,
             ts     INTEGER NOT NULL,
             open   REAL NOT NULL,
             high   REAL NOT NULL,
             low    REAL NOT NULL,
             close  REAL NOT NULL,
             volume REAL NOT NULL,
             PRIMARY KEY (ticker, ts)
         ) WITHOUT ROWID;",
    ).unwrap();

    let tx = conn.unchecked_transaction().unwrap();
    {
        let mut stmt = conn.prepare(
            "INSERT OR IGNORE INTO candles (ticker,ts,open,high,low,close,volume) VALUES (?1,?2,?3,?4,?5,?6,?7)"
        ).unwrap();
        for tid in 0..TICKERS {
            for bar in make_bars(tid) {
                stmt.execute(rusqlite::params![
                    tid as i64, bar.ts, bar.open as f64, bar.high as f64,
                    bar.low as f64, bar.close as f64, bar.volume as f64
                ]).unwrap();
            }
        }
    }
    tx.commit().unwrap();
    // Warm: force page cache by running one query
    let _: Vec<i64> = conn
        .prepare("SELECT ts FROM candles WHERE ticker=0 AND ts>=? AND ts<?")
        .unwrap()
        .query_map(rusqlite::params![BASE_TS, BASE_TS + BARS_PER_TICKER as i64 * 900], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    conn
}

fn bench_sqlite(conn: &Connection, ticker: usize) -> usize {
    let start = BASE_TS;
    let end = BASE_TS + BARS_PER_TICKER as i64 * 900;
    let mut stmt = conn.prepare_cached(
        "SELECT ts,open,high,low,close,volume FROM candles WHERE ticker=?1 AND ts>=?2 AND ts<?3 ORDER BY ts"
    ).unwrap();
    let rows: Vec<_> = stmt
        .query_map(rusqlite::params![ticker as i64, start, end], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    rows.len()
}

// ── Synapse-X setup ───────────────────────────────────────────────────────────

fn setup_smx(dir: &TempDir) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::with_capacity(TICKERS);
    for tid in 0..TICKERS {
        let path = dir.path().join(format!("ticker_{}.smx", tid));
        let mut s = Series::open(&path).unwrap();
        s.append(&make_bars(tid)).unwrap();
        s.close().unwrap();
        paths.push(path);
    }
    paths
}

fn bench_smx(path: &std::path::Path) -> usize {
    let mut s = Series::open(path).unwrap();
    let bars = s.range(BASE_TS..BASE_TS + BARS_PER_TICKER as i64 * 900).unwrap();
    bars.len()
}

/// Production-realistic: handle held open across queries.
fn bench_smx_held(s: &mut Series) -> usize {
    let bars = s.range(BASE_TS..BASE_TS + BARS_PER_TICKER as i64 * 900).unwrap();
    bars.len()
}

// ── timing helpers ────────────────────────────────────────────────────────────

fn percentile(mut v: Vec<u128>, p: f64) -> u128 {
    v.sort_unstable();
    let idx = ((p / 100.0) * (v.len() - 1) as f64).round() as usize;
    v[idx]
}

fn stats(times: &[u128]) -> (u128, u128, u128) {
    let mut v = times.to_vec();
    v.sort_unstable();
    let p50 = v[v.len() / 2];
    let p95 = v[(v.len() as f64 * 0.95) as usize];
    let mean = v.iter().sum::<u128>() / v.len() as u128;
    (p50, p95, mean)
}

fn main() {
    let sqlite_dir = TempDir::new().unwrap();
    let smx_dir = TempDir::new().unwrap();

    eprintln!("Setting up SQLite ({} tickers × {} bars)...", TICKERS, BARS_PER_TICKER);
    let conn = setup_sqlite(&sqlite_dir);
    eprintln!("Setting up Synapse-X...");
    let smx_paths = setup_smx(&smx_dir);

    // ── Warm cache pass ───────────────────────────────────────────────────────
    // Run each once before measuring
    for tid in 0..TICKERS {
        let _ = bench_sqlite(&conn, tid);
    }
    for path in &smx_paths {
        let _ = bench_smx(path);
    }

    // ── SQLite bench ──────────────────────────────────────────────────────────
    eprintln!("Benching SQLite ({} iters)...", ITERS);
    let mut sqlite_times = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let ticker = i % TICKERS;
        let t0 = Instant::now();
        let n = bench_sqlite(&conn, ticker);
        sqlite_times.push(t0.elapsed().as_micros());
        let _ = n;
    }

    // ── Synapse-X bench (open-each-query, realistic worst-case) ──────────────
    eprintln!("Benching Synapse-X open-each ({} iters)...", ITERS);
    let mut smx_times = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let path = &smx_paths[i % TICKERS];
        let t0 = Instant::now();
        let n = bench_smx(path);
        smx_times.push(t0.elapsed().as_micros());
        let _ = n;
    }

    // ── Synapse-X bench (handles held, production-realistic) ─────────────────
    eprintln!("Benching Synapse-X held-handle ({} iters)...", ITERS);
    let mut held: Vec<Series> = smx_paths.iter().map(|p| Series::open(p).unwrap()).collect();
    // warm
    for s in held.iter_mut() { let _ = bench_smx_held(s); }
    let mut smx_held_times = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let idx = i % TICKERS;
        let t0 = Instant::now();
        let n = bench_smx_held(&mut held[idx]);
        smx_held_times.push(t0.elapsed().as_nanos());
        let _ = n;
    }

    // ── Results ───────────────────────────────────────────────────────────────
    let (sql_p50, sql_p95, sql_mean) = stats(&sqlite_times);
    let (smx_p50, smx_p95, smx_mean) = stats(&smx_times);

    let speedup_p50 = sql_p50 as f64 / smx_p50.max(1) as f64;
    let speedup_mean = sql_mean as f64 / smx_mean.max(1) as f64;

    let gate_color = if speedup_p50 >= 10.0 {
        "GREEN ✅"
    } else if speedup_p50 >= 3.0 {
        "ORANGE ⚠️"
    } else {
        "RED ❌"
    };

    println!("\n=== W1 Candle-Range Bench Results ===");
    println!("Workload : {} tickers × {} bars, {} iters", TICKERS, BARS_PER_TICKER, ITERS);
    println!();
    println!("{:<20} {:>12} {:>12} {:>12}", "impl", "p50 (µs)", "p95 (µs)", "mean (µs)");
    println!("{}", "-".repeat(60));
    println!("{:<25} {:>12} {:>12} {:>12}", "SQLite-WITHOUT-ROWID", sql_p50, sql_p95, sql_mean);
    println!("{:<25} {:>12} {:>12} {:>12}", "Synapse-X (open-each)", smx_p50, smx_p95, smx_mean);
    // held times stored in ns
    let (h_p50, h_p95, h_mean) = stats(&smx_held_times);
    let h_p50_us = h_p50 as f64 / 1000.0;
    let h_p95_us = h_p95 as f64 / 1000.0;
    let h_mean_us = h_mean as f64 / 1000.0;
    println!("{:<25} {:>12.3} {:>12.3} {:>12.3}", "Synapse-X (held-handle)", h_p50_us, h_p95_us, h_mean_us);
    println!();
    let speedup_held_p50 = sql_p50 as f64 / h_p50_us.max(0.001);
    let speedup_held_mean = sql_mean as f64 / h_mean_us.max(0.001);
    println!("Speedup p50  (open-each)  : {:.1}×   mean : {:.1}×", speedup_p50, speedup_mean);
    println!("Speedup p50  (held-handle): {:.1}×   mean : {:.1}×", speedup_held_p50, speedup_held_mean);
    let gate_held = if speedup_held_p50 >= 10.0 { "GREEN ✅" } else if speedup_held_p50 >= 3.0 { "ORANGE ⚠️" } else { "RED ❌" };
    println!("Gate         (open-each)  : {}", gate_color);
    println!("Gate         (held-handle): {}", gate_held);

    // ── Write markdown report ─────────────────────────────────────────────────
    let md = format!(
        r#"# W1 Candle-Range Bench

## Results

| impl | p50 µs | p95 µs | mean µs |
|---|---:|---:|---:|
| SQLite WITHOUT ROWID | {} | {} | {} |
| Synapse-X mmap-pages | {} | {} | {} |

**Speedup p50: {:.1}×  (mean: {:.1}×)**

Gate: {}

## Setup
- {} tickers × {} bars (60d 15m candles)
- {} iterations, warm cache (SQLite page-cache=64MB, mmap warm)
- SQLite WITHOUT ROWID table, indexed on (ticker, ts)

## Notes
- Synapse-X: columnar mmap pages, delta-encoded ts (i32), f32 OHLCV
- Page size: 64KB, ~{} bars/page
- SQLite scans B-tree leaf pages; mmap scans sequential memory
"#,
        sql_p50, sql_p95, sql_mean,
        smx_p50, smx_p95, smx_mean,
        speedup_p50, speedup_mean,
        gate_color,
        TICKERS, BARS_PER_TICKER, ITERS,
        synapse_market::store::page::MAX_ROWS,
    );

    let report_path = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/w1_results.md");
    fs::write(report_path, &md).expect("write w1_results.md");
    println!("\nReport written to {}", report_path);
}
