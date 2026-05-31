use rusqlite::Connection;
/// W3 bench: SIMD aggregation kernels vs naive Rust vs SQLite vs DuckDB.
/// Workloads:
///   A1 — mean(close) 60d, 100 tickers, 1000 iters
///   A2 — VWAP 60d, 100 tickers, 1000 iters
///   A3 — rolling_mean_20 60d, 100 tickers, 1000 iters
///   A4 — pearson-corr 220×220 matrix, last 60d
///   A5 — ewma_20 60d, 100 tickers, 1000 iters
///
/// Compare: SIMD (SoA wide) | naive Rust loop | SQLite GROUP BY | DuckDB SQL
use std::time::{Duration, Instant};
use tempfile::TempDir;

use synapse_market::analytics;
use synapse_market::analytics::neon::scalar;
use synapse_market::store::page::Bar;

const TICKERS: usize = 100;
const BARS_PER_TICKER: usize = 2880; // 60d × 24h × 4 per hour (15m bars)
const ITERS: usize = 1000;
const CORR_TICKERS: usize = 220;
const BASE_TS: i64 = 1_700_000_000;
const EWMA_SPAN: usize = 20; // equiv alpha = 2/(20+1)
const ROLL_WINDOW: usize = 20;

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

// ── pre-extract close arrays ──────────────────────────────────────────────────

fn closes(bars: &[Bar]) -> Vec<f32> {
    bars.iter().map(|b| b.close).collect()
}

// ── SQLite setup ──────────────────────────────────────────────────────────────

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
             volume REAL    NOT NULL,
             high   REAL    NOT NULL,
             low    REAL    NOT NULL,
             PRIMARY KEY (ticker, ts)
         ) WITHOUT ROWID;",
    )
    .unwrap();
    {
        let mut stmt = conn
            .prepare(
                "INSERT INTO candles(ticker, ts, close, volume, high, low) VALUES(?,?,?,?,?,?)",
            )
            .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        for t in 0..TICKERS {
            for b in &make_bars(t) {
                stmt.execute(rusqlite::params![
                    t as i64,
                    b.ts,
                    b.close as f64,
                    b.volume as f64,
                    b.high as f64,
                    b.low as f64
                ])
                .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    conn
}

// ── DuckDB setup ──────────────────────────────────────────────────────────────

fn setup_duckdb(dir: &TempDir) -> duckdb::Connection {
    let path = dir.path().join("bench.duckdb");
    let conn = duckdb::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE candles (ticker INT, ts BIGINT, close FLOAT, volume FLOAT, high FLOAT, low FLOAT);").unwrap();
    {
        let mut app = conn.appender("candles").unwrap();
        for t in 0..TICKERS {
            for b in &make_bars(t) {
                app.append_row(duckdb::params![
                    t as i32, b.ts, b.close, b.volume, b.high, b.low
                ])
                .unwrap();
            }
        }
        app.flush().unwrap();
    }
    conn
}

// ── timing helper ─────────────────────────────────────────────────────────────

fn timed<T, F: Fn() -> T>(label: &str, iters: usize, f: F) -> Duration {
    let t0 = Instant::now();
    for _ in 0..iters {
        std::hint::black_box(f());
    }
    let elapsed = t0.elapsed();
    let per = elapsed / iters as u32;
    println!("  {label:<50} {per:>10?}/iter  total={elapsed:?}");
    elapsed
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let dir = TempDir::new().unwrap();
    let sqlite = setup_sqlite(&dir);

    // Build bar data once
    let all_bars: Vec<Vec<Bar>> = (0..TICKERS).map(make_bars).collect();
    let all_closes: Vec<Vec<f32>> = all_bars.iter().map(|b| closes(b)).collect();
    // SoA pre-extracted columns for VWAP
    let all_highs: Vec<Vec<f32>> = all_bars
        .iter()
        .map(|b| b.iter().map(|x| x.high).collect())
        .collect();
    let all_lows: Vec<Vec<f32>> = all_bars
        .iter()
        .map(|b| b.iter().map(|x| x.low).collect())
        .collect();
    let all_volumes: Vec<Vec<f32>> = all_bars
        .iter()
        .map(|b| b.iter().map(|x| x.volume).collect())
        .collect();

    // DuckDB setup
    let duckdb = setup_duckdb(&dir);

    println!("\n=== W3 SIMD AGG BENCH (SoA path) ===");

    // ── A1: mean(close) ───────────────────────────────────────────────────────
    println!(
        "\n[A1] mean(close) 60d × {} tickers × {} iters",
        TICKERS, ITERS
    );

    let t_simd_a1 = timed("SIMD mean_close_slice (SoA)", ITERS, || {
        all_closes
            .iter()
            .map(|c| analytics::mean_close_slice(c))
            .sum::<f32>()
    });

    let t_naive_a1 = timed("AoS mean_close strided (old path)", ITERS, || {
        all_bars
            .iter()
            .map(|bars| analytics::mean_close(bars))
            .sum::<f32>()
    });

    timed("SQLite AVG(close)", 100, || {
        let _: f64 = sqlite
            .query_row("SELECT AVG(close) FROM candles WHERE ticker=0", [], |r| {
                r.get(0)
            })
            .unwrap();
    });

    timed("DuckDB AVG(close)", 100, || {
        let mut stmt = duckdb
            .prepare("SELECT AVG(close) FROM candles WHERE ticker=0")
            .unwrap();
        let _: f64 = stmt.query_row([], |r| r.get(0)).unwrap();
    });

    let speedup_a1 = t_naive_a1.as_secs_f64() / t_simd_a1.as_secs_f64();
    println!("  → SIMD speedup vs naive: {speedup_a1:.1}×");

    // ── A2: VWAP ─────────────────────────────────────────────────────────────
    println!("\n[A2] VWAP 60d × {} tickers × {} iters", TICKERS, ITERS);

    let t_simd_a2 = timed("SIMD vwap_slices (SoA)", ITERS, || {
        (0..TICKERS)
            .map(|t| {
                analytics::vwap_slices(&all_highs[t], &all_lows[t], &all_closes[t], &all_volumes[t])
            })
            .sum::<f32>()
    });

    let t_naive_a2 = timed("AoS vwap (old path)", ITERS, || {
        all_bars.iter().map(|b| analytics::vwap(b)).sum::<f32>()
    });

    timed("SQLite VWAP", 100, || {
        let _: f64 = sqlite.query_row(
            "SELECT SUM((high+low+close)/3.0 * volume) / SUM(volume) FROM candles WHERE ticker=0",
            [], |r| r.get(0)
        ).unwrap();
    });

    timed("DuckDB VWAP", 100, || {
        let mut stmt = duckdb.prepare(
            "SELECT SUM((high+low+close)/3.0 * volume) / SUM(volume) FROM candles WHERE ticker=0"
        ).unwrap();
        let _: f64 = stmt.query_row([], |r| r.get(0)).unwrap();
    });

    let speedup_a2 = t_naive_a2.as_secs_f64() / t_simd_a2.as_secs_f64();
    println!("  → SIMD speedup vs naive: {speedup_a2:.1}×");

    // ── A3: rolling_mean_20 ───────────────────────────────────────────────────
    println!(
        "\n[A3] rolling_mean_20 60d × {} tickers × {} iters",
        TICKERS, ITERS
    );

    let t_simd_a3 = timed("SIMD rolling_mean_slice(20) (SoA)", ITERS, || {
        all_closes
            .iter()
            .map(|c| {
                analytics::rolling_mean_slice(c, ROLL_WINDOW)
                    .into_iter()
                    .sum::<f32>()
            })
            .sum::<f32>()
    });

    let t_naive_a3 = timed("AoS collect+rolling_mean (old path)", ITERS, || {
        all_bars
            .iter()
            .map(|b| {
                analytics::rolling_mean_close(b, ROLL_WINDOW)
                    .into_iter()
                    .sum::<f32>()
            })
            .sum::<f32>()
    });

    let speedup_a3 = t_naive_a3.as_secs_f64() / t_simd_a3.as_secs_f64();
    println!("  → SIMD speedup vs naive: {speedup_a3:.1}×");

    // ── A4: pearson 220×220 ───────────────────────────────────────────────────
    println!(
        "\n[A4] pearson corr {}×{} matrix (1 iter)",
        CORR_TICKERS, CORR_TICKERS
    );
    let corr_bars: Vec<Vec<Bar>> = (0..CORR_TICKERS).map(make_bars).collect();
    let corr_closes: Vec<Vec<f32>> = corr_bars.iter().map(|b| closes(b)).collect();

    let t0 = Instant::now();
    let mut matrix_simd = vec![0.0f32; CORR_TICKERS * CORR_TICKERS];
    for i in 0..CORR_TICKERS {
        for j in i..CORR_TICKERS {
            let r = analytics::pearson_correlation_slices(&corr_closes[i], &corr_closes[j]);
            matrix_simd[i * CORR_TICKERS + j] = r;
            matrix_simd[j * CORR_TICKERS + i] = r;
        }
    }
    let t_simd_a4 = t0.elapsed();
    println!("  SIMD corr-matrix {CORR_TICKERS}×{CORR_TICKERS} (SoA): {t_simd_a4:?}");
    assert!((matrix_simd[0] - 1.0).abs() < 1e-3, "diag != 1.0");

    let t0 = Instant::now();
    let mut matrix_naive = vec![0.0f32; CORR_TICKERS * CORR_TICKERS];
    for i in 0..CORR_TICKERS {
        for j in i..CORR_TICKERS {
            let r = std::hint::black_box(scalar::correlation_f32(&corr_closes[i], &corr_closes[j]));
            matrix_naive[i * CORR_TICKERS + j] = r;
            matrix_naive[j * CORR_TICKERS + i] = r;
        }
    }
    std::hint::black_box(&matrix_naive);
    let t_naive_a4 = t0.elapsed();
    println!("  naive corr-matrix {CORR_TICKERS}×{CORR_TICKERS}: {t_naive_a4:?}");

    let speedup_a4 = t_naive_a4.as_secs_f64() / t_simd_a4.as_secs_f64();
    println!("  → SIMD speedup vs naive: {speedup_a4:.1}×");

    // ── A5: ewma_20 ───────────────────────────────────────────────────────────
    println!("\n[A5] ewma_20 60d × {} tickers × {} iters", TICKERS, ITERS);
    let alpha = 2.0 / (EWMA_SPAN as f32 + 1.0);

    let t_simd_a5 = timed("SIMD ewma_slice(alpha) (SoA)", ITERS, || {
        all_closes
            .iter()
            .map(|c| analytics::ewma_slice(c, alpha).into_iter().sum::<f32>())
            .sum::<f32>()
    });

    let t_naive_a5 = timed("AoS collect+ewma_close (old path)", ITERS, || {
        all_bars
            .iter()
            .map(|b| analytics::ewma_close(b, alpha).into_iter().sum::<f32>())
            .sum::<f32>()
    });

    let speedup_a5 = t_naive_a5.as_secs_f64() / t_simd_a5.as_secs_f64();
    println!("  → SIMD speedup vs naive: {speedup_a5:.1}×");

    // ── Summary ───────────────────────────────────────────────────────────────
    println!("\n=== SUMMARY ===");
    let pass = |label: &str, actual: f64, threshold: f64| {
        let status = if actual >= threshold {
            "GREEN"
        } else {
            "RED  "
        };
        println!("  [{status}] {label:<40} {actual:.1}× (gate ≥{threshold:.0}×)");
    };
    pass("A1 mean(close) vs naive", speedup_a1, 4.0);
    pass("A2 VWAP vs naive", speedup_a2, 4.0);
    pass("A3 rolling_mean_20 vs naive", speedup_a3, 3.0);
    pass("A4 corr-matrix vs naive", speedup_a4, 10.0);
    pass("A5 ewma_20 vs naive", speedup_a5, 3.0);
    println!("  Stack estimate (7.2 × A1): {:.1}×", 7.2 * speedup_a1);
}
