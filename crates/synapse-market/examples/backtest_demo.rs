//! backtest_demo — synth 1 year × 10 tickers, MA-crossover strategy, regime-search.
//!
//! Run: cargo run --example backtest_demo -p synapse-market

use synapse_market::{Market, Strategy, Tick, Order, OrderSide};

const DAY: i64 = 86_400;
const YEAR_DAYS: usize = 252;

fn synth_ohlcv(seed: u64, days: usize) -> Vec<(i64, f64, f64, f64, f64, f64)> {
    let mut price = 100.0_f64;
    let mut rng = seed;
    let mut rows = Vec::with_capacity(days);
    for d in 0..days {
        // LCG pseudo-random
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let ret = ((rng >> 33) as f64 / u32::MAX as f64 - 0.5) * 0.04;
        let close = (price * (1.0 + ret)).max(1.0);
        let high = close * (1.0 + ((rng >> 40) as f64 / u32::MAX as f64) * 0.01);
        let low  = close * (1.0 - ((rng >> 48) as f64 / u32::MAX as f64) * 0.01);
        let open = price;
        let volume = 1_000_000.0 + ((rng >> 20) as f64 / u32::MAX as f64) * 500_000.0;
        rows.push((d as i64 * DAY, open, high, low, close, volume));
        price = close;
    }
    rows
}

/// Simple MA-crossover: buy when close > MA20, sell when close < MA10.
struct MaCrossover {
    prices: Vec<f64>,
    position: bool,
}

impl MaCrossover {
    fn new() -> Self { Self { prices: Vec::new(), position: false } }
    fn ma(&self, n: usize) -> f64 {
        let len = self.prices.len();
        if len < n { return 0.0; }
        self.prices[len - n..].iter().sum::<f64>() / n as f64
    }
}

impl Strategy for MaCrossover {
    fn on_tick(&mut self, tick: &Tick) -> Option<Order> {
        self.prices.push(tick.close);
        if self.prices.len() < 20 { return None; }
        let ma10 = self.ma(10);
        let ma20 = self.ma(20);
        if !self.position && tick.close > ma20 {
            self.position = true;
            Some(Order { side: OrderSide::Buy, qty: 100.0 })
        } else if self.position && tick.close < ma10 {
            self.position = false;
            Some(Order { side: OrderSide::Sell, qty: 100.0 })
        } else {
            None
        }
    }
}

fn main() -> anyhow::Result<()> {
    let m = Market::open_in_memory()?;
    let tickers = ["AAPL","MSFT","GOOG","AMZN","TSLA","NVDA","META","NFLX","AMD","INTC"];

    // --- Ingest 10 tickers × 252 days ---
    let t0 = std::time::Instant::now();
    let total_rows: usize = tickers.len() * YEAR_DAYS;
    for (i, &sym) in tickers.iter().enumerate() {
        let rows = synth_ohlcv(i as u64 * 12345 + 9999, YEAR_DAYS);
        m.ingest_ohlcv(sym, &rows)?;
        // Build regime embeddings
        synapse_market::regime::build_all(&m.conn, sym, &rows)?;
    }
    let ingest_ms = t0.elapsed().as_millis();
    let tps = total_rows as f64 / (t0.elapsed().as_secs_f64()).max(1e-9);
    println!("Ingest: {} rows in {}ms  →  {:.0} rows/sec", total_rows, ingest_ms, tps);

    // --- Ingest some news ---
    m.ingest_news(10 * DAY, "Apple beats earnings", "AAPL smashes Q4 estimates.", &["AAPL","MSFT"])?;
    m.ingest_news(50 * DAY, "Tech rally continues", "NVDA up 5% on AI demand.", &["NVDA","AMD"])?;

    // --- Regime search ---
    let t1 = std::time::Instant::now();
    let query_ts = 200 * DAY; // pick a day in the middle
    let similar = m.regime_search("AAPL", query_ts, 10)?;
    let regime_us = t1.elapsed().as_micros();
    println!("Regime search ({} past days): {}μs", similar.len(), regime_us);
    println!("  Top-3 similar days:");
    for (ts, sim) in similar.iter().take(3) {
        println!("    day={} sim={:.4}", ts / DAY, sim);
    }
    assert!(!similar.is_empty(), "regime_search returned empty");
    assert!(similar[0].1 >= 0.0 && similar[0].1 <= 1.001, "similarity out of range");

    // --- Backtest ---
    let t2 = std::time::Instant::now();
    let mut strat = MaCrossover::new();
    let report = m.backtest("AAPL", 0, YEAR_DAYS as i64 * DAY, &mut strat)?;
    let bt_ms = t2.elapsed().as_millis();
    println!("Backtest 1yr/AAPL: {}ms  ticks={} orders={} pnl={:.2} sharpe={:.3} mdd={:.2}%",
        bt_ms, report.ticks, report.orders, report.pnl, report.sharpe, report.max_drawdown * 100.0);
    assert_eq!(report.ticks, YEAR_DAYS);

    println!("\nAll assertions green.");
    Ok(())
}
