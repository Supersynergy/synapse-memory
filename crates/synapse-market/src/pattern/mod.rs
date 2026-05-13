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
    /// 13D filing within window_days.
    Filing13D { window_days: u32 },
    /// 8-K FDA meeting type + S-3 within window_days.
    FdaMeetingPlusS3 { window_days: u32 },
    /// Spinoff distribution date approaching within window_days.
    SpinoffApproaching { window_days: u32 },
    /// Float-squeeze: SI > si_min% AND CTB > ctb_bps_min bps.
    FloatSqueeze { si_min: f32, ctb_bps_min: u32 },
    /// Congress trade by member within window_days.
    CongressTrade { window_days: u32 },
    /// Matches if any toxic pattern present (reverse-split, dilution, China-OTC, etc.).
    SmartExcludeFail,
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
    Filing13D { ticker: String, ts: i64, percent_owned: f32 },
    FdaMeeting { ticker: String, ts: i64, meeting_type: String },
    S3Filing { ticker: String, ts: i64, raise_size: f64 },
    SpinoffAnnouncement { ticker: String, ts: i64, distribution_date: i64 },
    Squeeze { ticker: String, ts: i64, si_pct: f32, ctb_bps: u32 },
    CongressTradeEvent { ticker: String, ts: i64, amount_max: f64, member: String },
    ReverseSplit { ticker: String, ts: i64, ratio: f32 },
    DilutionRaise { ticker: String, ts: i64, n_recent_12mo: u32 },
}

impl Event {
    pub fn ticker(&self) -> &str {
        match self {
            Event::Candle(b) => {
                let _ = b;
                ""
            }
            Event::InsiderBuy { ticker, .. } => ticker,
            Event::Filing { ticker, .. } => ticker,
            Event::News { ticker, .. } => ticker,
            Event::Filing13D { ticker, .. } => ticker,
            Event::FdaMeeting { ticker, .. } => ticker,
            Event::S3Filing { ticker, .. } => ticker,
            Event::SpinoffAnnouncement { ticker, .. } => ticker,
            Event::Squeeze { ticker, .. } => ticker,
            Event::CongressTradeEvent { ticker, .. } => ticker,
            Event::ReverseSplit { ticker, .. } => ticker,
            Event::DilutionRaise { ticker, .. } => ticker,
        }
    }

    pub fn ts(&self) -> i64 {
        match self {
            Event::Candle(b) => b.ts,
            Event::InsiderBuy { ts, .. } => *ts,
            Event::Filing { ts, .. } => *ts,
            Event::News { ts, .. } => *ts,
            Event::Filing13D { ts, .. } => *ts,
            Event::FdaMeeting { ts, .. } => *ts,
            Event::S3Filing { ts, .. } => *ts,
            Event::SpinoffAnnouncement { ts, .. } => *ts,
            Event::Squeeze { ts, .. } => *ts,
            Event::CongressTradeEvent { ts, .. } => *ts,
            Event::ReverseSplit { ts, .. } => *ts,
            Event::DilutionRaise { ts, .. } => *ts,
        }
    }
}
