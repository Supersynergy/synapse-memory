//! Deterministic backtest replay.
//!
//! Usage:
//!   let report = market.backtest("AAPL", start, end, &mut strategy)?;

use crate::error::Result;
use crate::ohlcv::fetch_range;
use rusqlite::Connection;

/// A single OHLCV tick delivered to the strategy.
#[derive(Debug, Clone)]
pub struct Tick {
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub struct Order {
    pub side: OrderSide,
    /// Number of shares (fractional allowed).
    pub qty: f64,
}

/// Implement this to define a trading strategy.
pub trait Strategy {
    fn on_tick(&mut self, tick: &Tick) -> Option<Order>;
}

#[derive(Debug, Default)]
pub struct BacktestReport {
    pub ticks: usize,
    pub orders: usize,
    pub buys: usize,
    pub sells: usize,
    pub pnl: f64,
    pub max_drawdown: f64,
    pub sharpe: f64,
}

pub fn run<S: Strategy>(
    conn: &Connection,
    symbol: &str,
    start_ts: i64,
    end_ts: i64,
    strategy: &mut S,
) -> Result<BacktestReport> {
    let rows = fetch_range(conn, symbol, start_ts, end_ts)?;

    let mut position: f64 = 0.0;
    let mut cash: f64 = 100_000.0;
    let mut equity_curve: Vec<f64> = Vec::with_capacity(rows.len());
    let mut orders = 0usize;
    let mut buys = 0usize;
    let mut sells = 0usize;
    let mut daily_returns: Vec<f64> = Vec::with_capacity(rows.len());
    let mut prev_equity = cash;

    for &(ts, o, h, l, c, v) in &rows {
        let tick = Tick {
            ts,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
        };
        if let Some(ord) = strategy.on_tick(&tick) {
            let price = c; // fill at close (simplified)
            match ord.side {
                OrderSide::Buy => {
                    let cost = price * ord.qty;
                    if cash >= cost {
                        cash -= cost;
                        position += ord.qty;
                        buys += 1;
                        orders += 1;
                    }
                }
                OrderSide::Sell => {
                    let qty = ord.qty.min(position);
                    if qty > 0.0 {
                        cash += price * qty;
                        position -= qty;
                        sells += 1;
                        orders += 1;
                    }
                }
            }
        }
        let equity = cash + position * c;
        let ret = if prev_equity > 0.0 {
            equity / prev_equity - 1.0
        } else {
            0.0
        };
        daily_returns.push(ret);
        equity_curve.push(equity);
        prev_equity = equity;
    }

    let pnl = equity_curve.last().copied().unwrap_or(100_000.0) - 100_000.0;

    // Max drawdown
    let mut peak = f64::NEG_INFINITY;
    let mut max_drawdown = 0.0f64;
    for &e in &equity_curve {
        if e > peak {
            peak = e;
        }
        let dd = (peak - e) / peak.max(1.0);
        if dd > max_drawdown {
            max_drawdown = dd;
        }
    }

    // Sharpe (annualized, assume daily bars)
    let n = daily_returns.len() as f64;
    let mean = daily_returns.iter().sum::<f64>() / n.max(1.0);
    let variance = daily_returns
        .iter()
        .map(|r| (r - mean).powi(2))
        .sum::<f64>()
        / n.max(1.0);
    let std = variance.sqrt();
    let sharpe = if std > 1e-10 {
        mean / std * (252.0_f64).sqrt()
    } else {
        0.0
    };

    Ok(BacktestReport {
        ticks: rows.len(),
        orders,
        buys,
        sells,
        pnl,
        max_drawdown,
        sharpe,
    })
}
