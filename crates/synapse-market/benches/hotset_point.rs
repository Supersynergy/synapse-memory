use rusqlite::Connection;
/// HotSet point-lookup bench: 220 tickers × 122 bars.
/// Compares: SQLite WAL, Synapse-X cold, Synapse-X warm (HotSet).
use std::time::Instant;
use tempfile::TempDir;

use synapse_market::series::Series;
use synapse_market::store::page::Bar;

const TICKERS: usize = 220;
const BARS_PER_TICKER: usize = 122;
const ITERS: usize = 1000;
const BASE_TS: i64 = 1_700_000_000;
const STEP: i64 = 900;

fn make_bars(ticker_id: usize) -> Vec<Bar> {
    let price_base = 10.0 + ticker_id as f32 * 0.5;
    (0..BARS_PER_TICKER)
        .map(|i| Bar {
            ts: BASE_TS + i as i64 * STEP,
            open: price_base,
            high: price_base + 0.5,
            low: price_base - 0.5,
            close: price_base + 0.1,
            volume: 1000.0,
        })
        .collect()
}

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
             close  REAL    NOT NULL,
             PRIMARY KEY (ticker, ts)
         ) WITHOUT ROWID;",
    )
    .unwrap();
    {
        let mut stmt = conn
            .prepare("INSERT OR IGNORE INTO candles VALUES (?1,?2,?3)")
            .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        for t in 0..TICKERS {
            for b in make_bars(t) {
                stmt.execute(rusqlite::params![t as i64, b.ts, b.close as f64])
                    .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    conn
}

fn percentile(mut v: Vec<u128>, p: f64) -> u128 {
    v.sort_unstable();
    let idx = ((v.len() as f64 * p / 100.0) as usize).min(v.len() - 1);
    v[idx]
}

fn main() {
    let dir = TempDir::new().unwrap();

    // ── SQLite ────────────────────────────────────────────────────────────────
    let conn = setup_sqlite(&dir);
    let mut stmt = conn
        .prepare("SELECT close FROM candles WHERE ticker=?1 AND ts=?2")
        .unwrap();

    let mut sqlite_times: Vec<u128> = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let ticker = i % TICKERS;
        let bar_i = i % BARS_PER_TICKER;
        let ts = BASE_TS + bar_i as i64 * STEP;
        let t0 = Instant::now();
        let _: f64 = stmt
            .query_row(rusqlite::params![ticker as i64, ts], |r| r.get(0))
            .unwrap();
        sqlite_times.push(t0.elapsed().as_nanos());
    }

    // ── Synapse-X cold ────────────────────────────────────────────────────────
    let smx_dir = dir.path().join("smx");
    std::fs::create_dir_all(&smx_dir).unwrap();
    let mut series_vec: Vec<Series> = (0..TICKERS)
        .map(|t| {
            let mut s = Series::open(smx_dir.join(format!("{}.smx", t))).unwrap();
            s.append(&make_bars(t)).unwrap();
            s.flush_pending().unwrap();
            s
        })
        .collect();

    let mut cold_times: Vec<u128> = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let ticker = i % TICKERS;
        let bar_i = i % BARS_PER_TICKER;
        let ts = BASE_TS + bar_i as i64 * STEP;
        let t0 = Instant::now();
        let _ = series_vec[ticker].point_lookup(ts).unwrap();
        cold_times.push(t0.elapsed().as_nanos());
    }

    // ── Synapse-X warm (HotSet) ───────────────────────────────────────────────
    for s in series_vec.iter_mut() {
        s.enable_hot_cache(1000);
    }
    // 100-iter warmup
    for i in 0..100 {
        let ticker = i % TICKERS;
        let bar_i = i % BARS_PER_TICKER;
        let ts = BASE_TS + bar_i as i64 * STEP;
        let _ = series_vec[ticker].point_lookup(ts).unwrap();
    }
    let mut warm_times: Vec<u128> = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let ticker = i % TICKERS;
        let bar_i = i % BARS_PER_TICKER;
        let ts = BASE_TS + bar_i as i64 * STEP;
        let t0 = Instant::now();
        let _ = series_vec[ticker].point_lookup(ts).unwrap();
        warm_times.push(t0.elapsed().as_nanos());
    }

    // ── Report ────────────────────────────────────────────────────────────────
    let sqlite_p50 = percentile(sqlite_times.clone(), 50.0);
    let sqlite_p95 = percentile(sqlite_times, 95.0);
    let cold_p50 = percentile(cold_times.clone(), 50.0);
    let cold_p95 = percentile(cold_times, 95.0);
    let warm_p50 = percentile(warm_times.clone(), 50.0);
    let warm_p95 = percentile(warm_times, 95.0);

    println!(
        "=== hotset_point bench ({ITERS} iters, {TICKERS} tickers × {BARS_PER_TICKER} bars) ==="
    );
    println!(
        "SQLite WAL  p50={:>6}µs  p95={:>6}µs",
        sqlite_p50 / 1000,
        sqlite_p95 / 1000
    );
    println!(
        "Synapse cold p50={:>6}µs  p95={:>6}µs",
        cold_p50 / 1000,
        cold_p95 / 1000
    );
    println!(
        "Synapse warm p50={:>6}µs  p95={:>6}µs  [HotSet]",
        warm_p50 / 1000,
        warm_p95 / 1000
    );

    let gate = warm_p50 <= 12_000; // 12µs in nanos
    println!();
    println!(
        "Gate (warm p50 ≤ 12µs): {}",
        if gate { "PASS ✓" } else { "FAIL ✗" }
    );

    // Print hit/miss stats
    let mut total_hits = 0u64;
    let mut total_misses = 0u64;
    for s in &mut series_vec {
        if let Some(h) = s.hot.as_ref() {
            let (hits, misses) = h.stats();
            total_hits += hits;
            total_misses += misses;
        }
    }
    println!("HotSet hits={total_hits} misses={total_misses}");
}
