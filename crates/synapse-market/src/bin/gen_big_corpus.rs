/// Generate synthetic big-corpus for bloom-filter scale bench.
///
/// Writes 10 tickers × 1_000 pages (2_728 bars/page = 2_728_000 bars/ticker)
/// to ~/.synapse-x/big-corpus/<TICKER>.smx using a deterministic seed.
///
/// Usage: cargo run --bin gen_big_corpus
use std::path::PathBuf;
use synapse_market::series::Series;
use synapse_market::store::page::{Bar, MAX_ROWS};

const TICKERS: &[&str] = &[
    "AAPL", "MSFT", "GOOGL", "AMZN", "TSLA", "NVDA", "META", "BRK", "JPM", "V",
];
const PAGES_PER_TICKER: usize = 1_000;
const BARS_PER_TICKER: usize = PAGES_PER_TICKER * MAX_ROWS; // 2_728_000

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

fn main() -> std::io::Result<()> {
    let dir: PathBuf = {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".synapse-x").join("big-corpus")
    };
    std::fs::create_dir_all(&dir)?;

    let base_ts = 1_600_000_000i64; // 2020-09-13 ~UTC

    let t0 = std::time::Instant::now();
    let mut total_bytes: u64 = 0;

    for (ti, ticker) in TICKERS.iter().enumerate() {
        let path = dir.join(format!("{ticker}.smx"));
        // Remove existing so we start fresh
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.idx", path.display()));
        let _ = std::fs::remove_file(format!("{}.bloom", path.display()));

        let mut s = Series::open(&path)?;
        let bars = gen_bars(BARS_PER_TICKER, base_ts, (ti as u64 + 1) * 0xDEAD_BEEF);

        // Append in chunks of 100k to keep memory reasonable
        const CHUNK: usize = 100_000;
        let mut i = 0;
        while i < bars.len() {
            let end = (i + CHUNK).min(bars.len());
            s.append(&bars[i..end])?;
            i = end;
        }
        s.close()?;

        let meta = std::fs::metadata(&path)?;
        total_bytes += meta.len();
        eprintln!(
            "[{}/{}] {} — {:.1}MB ({} pages)",
            ti + 1,
            TICKERS.len(),
            ticker,
            meta.len() as f64 / 1_048_576.0,
            PAGES_PER_TICKER,
        );
    }

    eprintln!(
        "Done in {:.1}s — total {:.1}MB — dir: {}",
        t0.elapsed().as_secs_f64(),
        total_bytes as f64 / 1_048_576.0,
        dir.display(),
    );
    Ok(())
}
