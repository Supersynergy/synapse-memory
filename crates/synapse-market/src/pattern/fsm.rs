use super::{Event, Match, Pattern};
use std::collections::HashMap;

const SECS_PER_DAY: i64 = 86_400;

/// Per-pattern mutable FSM state.
#[derive(Debug, Clone)]
enum FsmState {
    DroughtBuy {
        quiet_days: u32,
        min_value: f64,
        last_buy_ts: Option<i64>,
    },
    InsiderCluster {
        k: u32,
        window_days: u32,
        buy_ts: Vec<i64>,
    },
    FdaTriple {
        window_days: u32,
        seen_8k: Option<i64>,
        seen_s3: Option<i64>,
        seen_phase3: Option<i64>,
    },
    VolumeSpike {
        multiplier: f32,
        window_bars: u32,
        ring: Vec<f32>,
        ring_pos: usize,
        filled: bool,
    },
    Filing13D {
        window_days: u32,
        seen_ts: Option<i64>,
    },
    FdaMeetingPlusS3 {
        window_days: u32,
        seen_meeting: Option<i64>,
        seen_s3: Option<i64>,
    },
    SpinoffApproaching {
        window_days: u32,
    },
    FloatSqueeze {
        si_min: f32,
        ctb_bps_min: u32,
    },
    CongressTrade {
        window_days: u32,
        seen_ts: Option<i64>,
    },
    SmartExcludeFail,
    And {
        left_id: u64,
        right_id: u64,
        left_match: Option<Match>,
        right_match: Option<Match>,
    },
    Or {
        left_id: u64,
        right_id: u64,
    },
    Then {
        a_id: u64,
        b_id: u64,
        within_bars: u32,
        a_match: Option<Match>,
        bars_since_a: u32,
    },
}

pub struct FsmEngine {
    patterns: Vec<(u64, Pattern)>,
    states: HashMap<u64, FsmState>,
}

impl Default for FsmEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FsmEngine {
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
            states: HashMap::new(),
        }
    }

    pub fn register(&mut self, p: Pattern) -> u64 {
        let id = p.id();
        let state = Self::init_state(&p, &mut self.states);
        self.states.insert(id, state);
        self.patterns.push((id, p));
        id
    }

    fn init_state(p: &Pattern, states: &mut HashMap<u64, FsmState>) -> FsmState {
        match p {
            Pattern::DroughtBuy {
                quiet_days,
                min_value,
            } => FsmState::DroughtBuy {
                quiet_days: *quiet_days,
                min_value: *min_value,
                last_buy_ts: None,
            },
            Pattern::InsiderCluster { k, window_days } => FsmState::InsiderCluster {
                k: *k,
                window_days: *window_days,
                buy_ts: Vec::new(),
            },
            Pattern::FdaTriple { window_days } => FsmState::FdaTriple {
                window_days: *window_days,
                seen_8k: None,
                seen_s3: None,
                seen_phase3: None,
            },
            Pattern::VolumeSpike {
                multiplier,
                window_bars,
            } => {
                let w = *window_bars as usize;
                FsmState::VolumeSpike {
                    multiplier: *multiplier,
                    window_bars: *window_bars,
                    ring: vec![0.0; w],
                    ring_pos: 0,
                    filled: false,
                }
            }
            Pattern::Filing13D { window_days } => FsmState::Filing13D {
                window_days: *window_days,
                seen_ts: None,
            },
            Pattern::FdaMeetingPlusS3 { window_days } => FsmState::FdaMeetingPlusS3 {
                window_days: *window_days,
                seen_meeting: None,
                seen_s3: None,
            },
            Pattern::SpinoffApproaching { window_days } => FsmState::SpinoffApproaching {
                window_days: *window_days,
            },
            Pattern::FloatSqueeze {
                si_min,
                ctb_bps_min,
            } => FsmState::FloatSqueeze {
                si_min: *si_min,
                ctb_bps_min: *ctb_bps_min,
            },
            Pattern::CongressTrade { window_days } => FsmState::CongressTrade {
                window_days: *window_days,
                seen_ts: None,
            },
            Pattern::SmartExcludeFail => FsmState::SmartExcludeFail,
            Pattern::And(l, r) => {
                let lid = l.id();
                let rid = r.id();
                let ls = Self::init_state(l, states);
                states.insert(lid, ls);
                let rs = Self::init_state(r, states);
                states.insert(rid, rs);
                FsmState::And {
                    left_id: lid,
                    right_id: rid,
                    left_match: None,
                    right_match: None,
                }
            }
            Pattern::Or(l, r) => {
                let lid = l.id();
                let rid = r.id();
                let ls = Self::init_state(l, states);
                states.insert(lid, ls);
                let rs = Self::init_state(r, states);
                states.insert(rid, rs);
                FsmState::Or {
                    left_id: lid,
                    right_id: rid,
                }
            }
            Pattern::Then(a, b, within) => {
                let aid = a.id();
                let bid = b.id();
                let a_s = Self::init_state(a, states);
                states.insert(aid, a_s);
                let b_s = Self::init_state(b, states);
                states.insert(bid, b_s);
                FsmState::Then {
                    a_id: aid,
                    b_id: bid,
                    within_bars: *within,
                    a_match: None,
                    bars_since_a: 0,
                }
            }
        }
    }

    /// Feed one event for a specific ticker. Returns matches triggered.
    pub fn on_event(&mut self, ticker: &str, event: &Event) -> Vec<Match> {
        let ts = event.ts();
        let ids: Vec<u64> = self.patterns.iter().map(|(id, _)| *id).collect();
        let mut out = Vec::new();
        for id in ids {
            let matches = Self::step(id, ticker, ts, event, &mut self.states);
            out.extend(matches);
        }
        out
    }

    /// Drain any remaining partial matches (e.g., end-of-stream).
    pub fn flush(&mut self) -> Vec<Match> {
        Vec::new()
    }

    fn step(
        id: u64,
        ticker: &str,
        ts: i64,
        event: &Event,
        states: &mut HashMap<u64, FsmState>,
    ) -> Vec<Match> {
        // Take ownership to avoid borrow conflicts when recursing for child IDs
        let Some(mut state) = states.remove(&id) else {
            return vec![];
        };
        let result = Self::step_state(id, &mut state, ticker, ts, event, states);
        states.insert(id, state);
        result
    }

    fn step_state(
        id: u64,
        state: &mut FsmState,
        ticker: &str,
        ts: i64,
        event: &Event,
        states: &mut HashMap<u64, FsmState>,
    ) -> Vec<Match> {
        match state {
            FsmState::DroughtBuy {
                quiet_days,
                min_value,
                last_buy_ts,
            } => {
                let qd = *quiet_days as i64;
                let mv = *min_value;
                if let Event::InsiderBuy {
                    value_usd,
                    ts: buy_ts,
                    ticker: t,
                } = event
                {
                    if t != ticker {
                        return vec![];
                    }
                    let prev = *last_buy_ts;
                    *last_buy_ts = Some(*buy_ts);
                    if let Some(prev_ts) = prev {
                        let gap_days = (buy_ts - prev_ts) / SECS_PER_DAY;
                        if gap_days >= qd && *value_usd >= mv {
                            return vec![Match {
                                pattern_id: id,
                                ticker: ticker.to_string(),
                                ts_start: prev_ts,
                                ts_end: *buy_ts,
                                confidence: 1.0,
                                catalyst_ids: vec![],
                            }];
                        }
                    }
                }
                vec![]
            }
            FsmState::InsiderCluster {
                k,
                window_days,
                buy_ts,
            } => {
                let k_needed = *k;
                let window = *window_days as i64 * SECS_PER_DAY;
                if let Event::InsiderBuy {
                    ts: bt, ticker: t, ..
                } = event
                {
                    if t != ticker {
                        return vec![];
                    }
                    buy_ts.push(*bt);
                    // prune old
                    buy_ts.retain(|&t| *bt - t <= window);
                    if buy_ts.len() >= k_needed as usize {
                        let ts_start = *buy_ts.iter().min().unwrap();
                        let ts_end = *buy_ts.iter().max().unwrap();
                        return vec![Match {
                            pattern_id: id,
                            ticker: ticker.to_string(),
                            ts_start,
                            ts_end,
                            confidence: 1.0,
                            catalyst_ids: vec![],
                        }];
                    }
                }
                vec![]
            }
            FsmState::FdaTriple {
                window_days,
                seen_8k,
                seen_s3,
                seen_phase3,
            } => {
                let window = *window_days as i64 * SECS_PER_DAY;
                if let Event::Filing {
                    kind,
                    ticker: t,
                    ts: fts,
                } = event
                {
                    if t != ticker {
                        return vec![];
                    }
                    match kind.as_str() {
                        "8-K" => *seen_8k = Some(*fts),
                        "S-3" => *seen_s3 = Some(*fts),
                        "Phase-III" => *seen_phase3 = Some(*fts),
                        _ => {}
                    }
                    // check all three within window
                    if let (Some(t1), Some(t2), Some(t3)) = (*seen_8k, *seen_s3, *seen_phase3) {
                        let earliest = t1.min(t2).min(t3);
                        let latest = t1.max(t2).max(t3);
                        if latest - earliest <= window {
                            return vec![Match {
                                pattern_id: id,
                                ticker: ticker.to_string(),
                                ts_start: earliest,
                                ts_end: latest,
                                confidence: 1.0,
                                catalyst_ids: vec![],
                            }];
                        }
                    }
                }
                vec![]
            }
            FsmState::VolumeSpike {
                multiplier,
                window_bars,
                ring,
                ring_pos,
                filled,
            } => {
                let mult = *multiplier;
                let w = *window_bars as usize;
                if let Event::Candle(bar) = event {
                    // Compute mean over existing ring BEFORE inserting current bar
                    let prev_filled = *filled;
                    let mean: f32 = if prev_filled {
                        ring.iter().sum::<f32>() / w as f32
                    } else {
                        0.0
                    };
                    let pos = *ring_pos % w;
                    ring[pos] = bar.volume;
                    *ring_pos += 1;
                    if !*filled && *ring_pos >= w {
                        *filled = true;
                    }
                    if prev_filled && mean > 0.0 && bar.volume > mult * mean {
                        return vec![Match {
                            pattern_id: id,
                            ticker: ticker.to_string(),
                            ts_start: bar.ts,
                            ts_end: bar.ts,
                            confidence: (bar.volume / (mult * mean)).min(1.0),
                            catalyst_ids: vec![],
                        }];
                    }
                }
                vec![]
            }
            FsmState::Filing13D {
                window_days,
                seen_ts,
            } => {
                if let Event::Filing13D {
                    ticker: t, ts: fts, ..
                } = event
                {
                    if t != ticker {
                        return vec![];
                    }
                    let prev = *seen_ts;
                    *seen_ts = Some(*fts);
                    if let Some(prev_ts) = prev {
                        let window = *window_days as i64 * SECS_PER_DAY;
                        if fts - prev_ts <= window {
                            return vec![Match {
                                pattern_id: id,
                                ticker: ticker.to_string(),
                                ts_start: prev_ts,
                                ts_end: *fts,
                                confidence: 1.0,
                                catalyst_ids: vec![],
                            }];
                        }
                    }
                }
                vec![]
            }
            FsmState::FdaMeetingPlusS3 {
                window_days,
                seen_meeting,
                seen_s3,
            } => {
                let window = *window_days as i64 * SECS_PER_DAY;
                match event {
                    Event::FdaMeeting {
                        ticker: t, ts: fts, ..
                    } if t == ticker => {
                        *seen_meeting = Some(*fts);
                    }
                    Event::S3Filing {
                        ticker: t, ts: fts, ..
                    } if t == ticker => {
                        *seen_s3 = Some(*fts);
                    }
                    _ => {}
                }
                if let (Some(tm), Some(ts3)) = (*seen_meeting, *seen_s3)
                    && (tm - ts3).abs() <= window
                {
                    return vec![Match {
                        pattern_id: id,
                        ticker: ticker.to_string(),
                        ts_start: tm.min(ts3),
                        ts_end: tm.max(ts3),
                        confidence: 1.0,
                        catalyst_ids: vec![],
                    }];
                }
                vec![]
            }
            FsmState::SpinoffApproaching { window_days } => {
                let window = *window_days as i64 * SECS_PER_DAY;
                if let Event::SpinoffAnnouncement {
                    ticker: t,
                    ts: ann_ts,
                    distribution_date,
                } = event
                {
                    if t != ticker {
                        return vec![];
                    }
                    let days_until = distribution_date - ann_ts;
                    if days_until >= 0 && days_until <= window {
                        return vec![Match {
                            pattern_id: id,
                            ticker: ticker.to_string(),
                            ts_start: *ann_ts,
                            ts_end: *distribution_date,
                            confidence: 1.0 - (days_until as f32 / window as f32),
                            catalyst_ids: vec![],
                        }];
                    }
                }
                vec![]
            }
            FsmState::FloatSqueeze {
                si_min,
                ctb_bps_min,
            } => {
                if let Event::Squeeze {
                    ticker: t,
                    ts: sts,
                    si_pct,
                    ctb_bps,
                } = event
                {
                    if t != ticker {
                        return vec![];
                    }
                    if *si_pct >= *si_min && *ctb_bps >= *ctb_bps_min {
                        return vec![Match {
                            pattern_id: id,
                            ticker: ticker.to_string(),
                            ts_start: *sts,
                            ts_end: *sts,
                            confidence: (*si_pct / 100.0).min(1.0),
                            catalyst_ids: vec![],
                        }];
                    }
                }
                vec![]
            }
            FsmState::CongressTrade {
                window_days,
                seen_ts,
            } => {
                if let Event::CongressTradeEvent {
                    ticker: t, ts: cts, ..
                } = event
                {
                    if t != ticker {
                        return vec![];
                    }
                    let prev = *seen_ts;
                    *seen_ts = Some(*cts);
                    if let Some(prev_ts) = prev {
                        let window = *window_days as i64 * SECS_PER_DAY;
                        if cts - prev_ts <= window {
                            return vec![Match {
                                pattern_id: id,
                                ticker: ticker.to_string(),
                                ts_start: prev_ts,
                                ts_end: *cts,
                                confidence: 1.0,
                                catalyst_ids: vec![],
                            }];
                        }
                    }
                }
                vec![]
            }
            FsmState::SmartExcludeFail => {
                match event {
                    Event::ReverseSplit { ticker: t, .. }
                    | Event::DilutionRaise { ticker: t, .. }
                        if t == ticker =>
                    {
                        return vec![Match {
                            pattern_id: id,
                            ticker: ticker.to_string(),
                            ts_start: ts,
                            ts_end: ts,
                            confidence: 1.0,
                            catalyst_ids: vec![],
                        }];
                    }
                    _ => {}
                }
                vec![]
            }
            FsmState::And {
                left_id,
                right_id,
                left_match,
                right_match,
            } => {
                let lid = *left_id;
                let rid = *right_id;
                let lm = Self::step(lid, ticker, ts, event, states);
                let rm = Self::step(rid, ticker, ts, event, states);
                if let Some(m) = lm.into_iter().next() {
                    *left_match = Some(m);
                }
                if let Some(m) = rm.into_iter().next() {
                    *right_match = Some(m);
                }
                if left_match.is_some() && right_match.is_some() {
                    let lmatch = left_match.take().unwrap();
                    let rmatch = right_match.take().unwrap();
                    return vec![Match {
                        pattern_id: id,
                        ticker: ticker.to_string(),
                        ts_start: lmatch.ts_start.min(rmatch.ts_start),
                        ts_end: lmatch.ts_end.max(rmatch.ts_end),
                        confidence: (lmatch.confidence + rmatch.confidence) / 2.0,
                        catalyst_ids: vec![],
                    }];
                }
                vec![]
            }
            FsmState::Or { left_id, right_id } => {
                let lid = *left_id;
                let rid = *right_id;
                let mut lm = Self::step(lid, ticker, ts, event, states);
                let rm = Self::step(rid, ticker, ts, event, states);
                lm.extend(rm);
                for m in &mut lm {
                    m.pattern_id = id;
                }
                lm
            }
            FsmState::Then {
                a_id,
                b_id,
                within_bars,
                a_match,
                bars_since_a,
            } => {
                let aid = *a_id;
                let bid = *b_id;
                let wb = *within_bars;
                // tick bar counter
                if matches!(event, Event::Candle(_)) && a_match.is_some() {
                    *bars_since_a += 1;
                    if *bars_since_a > wb {
                        *a_match = None;
                        *bars_since_a = 0;
                    }
                }
                let am = Self::step(aid, ticker, ts, event, states);
                let bm = Self::step(bid, ticker, ts, event, states);
                if let Some(m) = am.into_iter().next() {
                    *a_match = Some(m);
                    *bars_since_a = 0;
                }
                if let Some(bm) = bm.into_iter().next()
                    && let Some(am) = a_match.take()
                {
                    *bars_since_a = 0;
                    return vec![Match {
                        pattern_id: id,
                        ticker: ticker.to_string(),
                        ts_start: am.ts_start,
                        ts_end: bm.ts_end,
                        confidence: (am.confidence + bm.confidence) / 2.0,
                        catalyst_ids: vec![],
                    }];
                }
                vec![]
            }
        }
    }
}
