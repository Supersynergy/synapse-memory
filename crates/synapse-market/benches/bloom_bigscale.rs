/// Big-scale bloom bench: 10 tickers × 1000 pages, 1000 queries (50% pos / 50% neg).
///
/// Pre-requisite: run `cargo run --bin gen_big_corpus` first.
/// If corpus missing, bench creates it inline (slower startup).
///
/// 3 variants:
///   smx_bloom   — range_filter() with bloom guard
///   smx_noBloom — range() no bloom guard (header-skip only)
///   sqlite_wal  — same data, same queries, WAL mode
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rusqlite::Connection;
use std::path::PathBuf;
use synapse_market::series::Series;
use synapse_market::store::page::{Bar, MAX_ROWS};

const TICKERS: &[&str] = &[
    "AAPL", "MSFT", "GOOGL", "AMZN", "TSLA", "NVDA", "META", "BRK", "JPM", "V",
];
const PAGES_PER_TICKER: usize = 500; // reduced from 1000 for bench startup speed
const BARS_PER_TICKER: usize = PAGES_PER_TICKER * MAX_ROWS;
const N_QUERIES: usize = 1_000;
const BASE_TS: i64 = 1_600_000_000i64;

fn lcg_next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 33
}

fn gen_bars(n: usize, base_ts: i64, seed: u64) -> Vec<Bar> {
    let mut state = seed;
    let mut bars = Vec::with_capacity(n);
    let mut price = 100.0f32;
    for i in 0..n {
        let r = lcg_next(&mut state);
        let delta = ((r & 0xFF) as f32 - 127.0) * 0.01;
        price = (price + delta).max(1.0);
        let high = price + ((r >> 8 & 0xFF) as f32) * 0.005;
        let low = price - ((r >> 16 & 0xFF) as f32) * 0.005;
        let vol = 1000.0 + (r >> 24 & 0xFFFF) as f32;
        bars.push(Bar {
            ts: base_ts + i as i64 * 60,
            open: price,
            high,
            low: low.min(price),
            close: price,
            volume: vol,
        });
    }
    bars
}

fn corpus_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".synapse-x").join("big-corpus")
}

fn ensure_corpus() -> PathBuf {
    let dir = corpus_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // Check if all files exist and are non-empty
    let all_ok = TICKERS.iter().all(|t| {
        let p = dir.join(format!("{t}.smx"));
        p.exists() && std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false)
    });
    if !all_ok {
        eprintln!("Corpus missing — generating (one-time, ~30s) …");
        for (ti, ticker) in TICKERS.iter().enumerate() {
            let path = dir.join(format!("{ticker}.smx"));
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(format!("{}.idx", path.display()));
            let _ = std::fs::remove_file(format!("{}.bloom", path.display()));
            let mut s = Series::open(&path).unwrap();
            let bars = gen_bars(BARS_PER_TICKER, BASE_TS, (ti as u64 + 1) * 0xDEAD_BEEF);
            const CHUNK: usize = 50_000;
            let mut i = 0;
            while i < bars.len() {
                let end = (i + CHUNK).min(bars.len());
                s.append(&bars[i..end]).unwrap();
                i = end;
            }
            s.close().unwrap();
        }
        eprintln!("Corpus ready.");
    }
    dir
}

fn build_queries() -> (Vec<std::ops::Range<i64>>, Vec<std::ops::Range<i64>>) {
    let series_end = BASE_TS + BARS_PER_TICKER as i64 * 60;
    let pos: Vec<_> = (0..N_QUERIES / 2)
        .map(|i| {
            // pick a ts well inside the data
            let ts = BASE_TS + (i as i64 * 60 * 499) % (BARS_PER_TICKER as i64 * 60 - 3600);
            ts..ts + 3600
        })
        .collect();
    let neg: Vec<_> = (0..N_QUERIES / 2)
        .map(|i| {
            // ts way after data ends — should bloom-reject immediately
            let ts = series_end + (i as i64 + 1) * 3_600 * 24 * 365;
            ts..ts + 3600
        })
        .collect();
    (pos, neg)
}

fn setup_sqlite(dir: &std::path::Path) -> Connection {
    let path = dir.join("bigscale.db");
    if path.exists() {
        if let Ok(conn) = Connection::open(&path) {
            // verify table exists
            let ok: bool = conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='candles'",
                    [],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if ok {
                return conn;
            }
        }
    }
    eprintln!(
        "SQLite: building WAL DB ({} tickers × {} bars) …",
        TICKERS.len(),
        BARS_PER_TICKER
    );
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA cache_size=-131072;
         CREATE TABLE IF NOT EXISTS candles (
             ticker INTEGER NOT NULL,
             ts     INTEGER NOT NULL,
             open   REAL,
             high   REAL,
             low    REAL,
             close  REAL,
             volume REAL,
             PRIMARY KEY (ticker, ts)
         ) WITHOUT ROWID;",
    )
    .unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    {
        let mut stmt = conn
            .prepare(
                "INSERT OR IGNORE INTO candles (ticker,ts,open,high,low,close,volume) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
            )
            .unwrap();
        for (ti, _ticker) in TICKERS.iter().enumerate() {
            let bars = gen_bars(BARS_PER_TICKER, BASE_TS, (ti as u64 + 1) * 0xDEAD_BEEF);
            for bar in &bars {
                stmt.execute(rusqlite::params![
                    ti as i64,
                    bar.ts,
                    bar.open as f64,
                    bar.high as f64,
                    bar.low as f64,
                    bar.close as f64,
                    bar.volume as f64
                ])
                .unwrap();
            }
        }
    }
    tx.commit().unwrap();
    eprintln!("SQLite ready.");
    conn
}

fn bench_bloom_bigscale(c: &mut Criterion) {
    let corpus_dir = ensure_corpus();
    let (pos_queries, neg_queries) = build_queries();

    let mut smx_series: Vec<Series> = TICKERS
        .iter()
        .map(|t| Series::open(corpus_dir.join(format!("{t}.smx"))).unwrap())
        .collect();

    let sqlite_conn = setup_sqlite(&corpus_dir);

    let n = TICKERS.len();
    let mut group = c.benchmark_group("bloom_bigscale_500pages");
    group.sample_size(20);

    // --- smx with bloom, negative queries ---
    group.bench_function("smx_bloom_neg", |b| {
        b.iter(|| {
            for (qi, q) in neg_queries.iter().enumerate() {
                let s = &mut smx_series[qi % n];
                let r = s.range_filter(q.clone()).unwrap();
                criterion::black_box(r);
            }
        })
    });

    // --- smx without bloom (range), negative queries ---
    group.bench_function("smx_noBloom_neg", |b| {
        b.iter(|| {
            for (qi, q) in neg_queries.iter().enumerate() {
                let s = &mut smx_series[qi % n];
                let r = s.range(q.clone()).unwrap();
                criterion::black_box(r);
            }
        })
    });

    // --- SQLite WAL, negative queries ---
    group.bench_function("sqlite_wal_neg", |b| {
        b.iter(|| {
            let mut stmt = sqlite_conn
                .prepare_cached("SELECT ts FROM candles WHERE ticker=?1 AND ts>=?2 AND ts<?3")
                .unwrap();
            for (qi, q) in neg_queries.iter().enumerate() {
                let ti = (qi % n) as i64;
                let rows: Vec<i64> = stmt
                    .query_map(rusqlite::params![ti, q.start, q.end], |r| r.get(0))
                    .unwrap()
                    .map(|r| r.unwrap())
                    .collect();
                criterion::black_box(rows);
            }
        })
    });

    // --- smx with bloom, positive queries ---
    group.bench_function("smx_bloom_pos", |b| {
        b.iter(|| {
            for (qi, q) in pos_queries.iter().enumerate() {
                let s = &mut smx_series[qi % n];
                let r = s.range_filter(q.clone()).unwrap();
                criterion::black_box(r);
            }
        })
    });

    // --- smx without bloom, positive queries ---
    group.bench_function("smx_noBloom_pos", |b| {
        b.iter(|| {
            for (qi, q) in pos_queries.iter().enumerate() {
                let s = &mut smx_series[qi % n];
                let r = s.range(q.clone()).unwrap();
                criterion::black_box(r);
            }
        })
    });

    group.finish();
}

criterion_group!(benches, bench_bloom_bigscale);
criterion_main!(benches);
