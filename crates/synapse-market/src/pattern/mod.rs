pub mod fsm;
pub mod dsl;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Composable pattern AST.
#[derive(Debug, Clone)]
pub enum Pattern {
    /// No insider activity for `quiet_days` days, then a buy ≥ `min_value` USD.
    DroughtBuy { quiet_days: u32, min_value: f64 },
    /// K insider buys within `window_days` days.
    InsiderCluster { k: u32, window_days: u32 },
    /// 8-K + S-3 + Phase-III filing all within `window_days` days.
    FdaTriple { window_days: u32 },
    /// Volume > `multiplier` × rolling mean over `window_bars` bars.
    VolumeSpike { multiplier: f32, window_bars: u32 },
    /// Both sub-patterns must match.
    And(Box<Pattern>, Box<Pattern>),
    /// Either sub-pattern must match.
    Or(Box<Pattern>, Box<Pattern>),
    /// Pattern A must match, then pattern B within `within_bars` bars.
    Then(Box<Pattern>, Box<Pattern>, u32),
}

impl Pattern {
    pub fn id(&self) -> u64 {
        let mut h = DefaultHasher::new();
        format!("{:?}", self).hash(&mut h);
        h.finish()
    }
}

/// A detected pattern match.
#[derive(Debug, Clone)]
pub struct Match {
    pub pattern_id: u64,
    pub ticker: String,
    pub ts_start: i64,
    pub ts_end: i64,
    pub confidence: f32,
    pub catalyst_ids: Vec<u64>,
}

/// Events fed to the FSM engine.
#[derive(Debug, Clone)]
pub enum Event {
    Candle(crate::store::page::Bar),
    InsiderBuy { value_usd: f64, ts: i64, ticker: String },
    Filing { kind: String, ticker: String, ts: i64 },
    News { headline: String, ts: i64, ticker: String },
}

impl Event {
    pub fn ticker(&self) -> &str {
        match self {
            Event::Candle(b) => {
                // Bar doesn't carry ticker; caller must set context per ticker
                let _ = b;
                ""
            }
            Event::InsiderBuy { ticker, .. } => ticker,
            Event::Filing { ticker, .. } => ticker,
            Event::News { ticker, .. } => ticker,
        }
    }

    pub fn ts(&self) -> i64 {
        match self {
            Event::Candle(b) => b.ts,
            Event::InsiderBuy { ts, .. } => *ts,
            Event::Filing { ts, .. } => *ts,
            Event::News { ts, .. } => *ts,
        }
    }
}
