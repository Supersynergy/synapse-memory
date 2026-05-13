use synapse_market::pattern::{Pattern, Event};
use synapse_market::pattern::fsm::FsmEngine;

const SECS: i64 = 86_400;

#[test]
fn test_filing_13d_fires() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::Filing13D { window_days: 30 });

    let t = "ACME";
    // First filing — sets seen_ts
    let ev1 = Event::Filing13D { ticker: t.into(), ts: 0, percent_owned: 10.0 };
    let m1 = engine.on_event(t, &ev1);
    assert!(m1.is_empty(), "first 13D — no prior");

    // Second within window
    let ev2 = Event::Filing13D { ticker: t.into(), ts: 10 * SECS, percent_owned: 12.0 };
    let m2 = engine.on_event(t, &ev2);
    assert_eq!(m2.len(), 1);
    assert_eq!(m2[0].pattern_id, id);
}

#[test]
fn test_fda_meeting_plus_s3_fires() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::FdaMeetingPlusS3 { window_days: 60 });

    let t = "RVMD";
    engine.on_event(t, &Event::FdaMeeting { ticker: t.into(), ts: 0, meeting_type: "PDUFA".into() });
    let m = engine.on_event(t, &Event::S3Filing { ticker: t.into(), ts: 20 * SECS, raise_size: 50_000_000.0 });
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].pattern_id, id);
}

#[test]
fn test_spinoff_approaching_fires() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::SpinoffApproaching { window_days: 30 });

    let t = "GE";
    let now: i64 = 1_000_000;
    let distrib = now + 15 * SECS; // 15 days away — within 30-day window
    let m = engine.on_event(t, &Event::SpinoffAnnouncement {
        ticker: t.into(), ts: now, distribution_date: distrib,
    });
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].pattern_id, id);
}

#[test]
fn test_float_squeeze_fires() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::FloatSqueeze { si_min: 20.0, ctb_bps_min: 50 });

    let t = "GME";
    let m = engine.on_event(t, &Event::Squeeze {
        ticker: t.into(), ts: 0, si_pct: 25.0, ctb_bps: 120,
    });
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].pattern_id, id);
}

#[test]
fn test_float_squeeze_no_fire_below_threshold() {
    let mut engine = FsmEngine::new();
    engine.register(Pattern::FloatSqueeze { si_min: 20.0, ctb_bps_min: 50 });

    let t = "GME";
    let m = engine.on_event(t, &Event::Squeeze {
        ticker: t.into(), ts: 0, si_pct: 10.0, ctb_bps: 20,
    });
    assert!(m.is_empty());
}

#[test]
fn test_smart_exclude_fail_on_reverse_split() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::SmartExcludeFail);

    let t = "BTBT";
    let m = engine.on_event(t, &Event::ReverseSplit { ticker: t.into(), ts: 0, ratio: 10.0 });
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].pattern_id, id);
}

#[test]
fn test_smart_exclude_fail_on_dilution() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::SmartExcludeFail);

    let t = "BBIG";
    let m = engine.on_event(t, &Event::DilutionRaise { ticker: t.into(), ts: 0, n_recent_12mo: 3 });
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].pattern_id, id);
}

#[test]
fn test_compose_filing13d_and_float_squeeze() {
    let p = Pattern::And(
        Box::new(Pattern::Filing13D { window_days: 30 }),
        Box::new(Pattern::FloatSqueeze { si_min: 15.0, ctb_bps_min: 30 }),
    );
    let mut engine = FsmEngine::new();
    let id = engine.register(p);

    let t = "XYZ";
    // Satisfy 13D (two filings within window)
    engine.on_event(t, &Event::Filing13D { ticker: t.into(), ts: 0, percent_owned: 8.0 });
    engine.on_event(t, &Event::Filing13D { ticker: t.into(), ts: 5 * SECS, percent_owned: 10.0 });

    // Satisfy squeeze
    let m = engine.on_event(t, &Event::Squeeze { ticker: t.into(), ts: 6 * SECS, si_pct: 20.0, ctb_bps: 80 });
    assert!(m.iter().any(|x| x.pattern_id == id), "And(13D, FloatSqueeze) should fire");
}

#[test]
fn test_compose_then_congress_then_squeeze() {
    let p = Pattern::Then(
        Box::new(Pattern::CongressTrade { window_days: 14 }),
        Box::new(Pattern::FloatSqueeze { si_min: 10.0, ctb_bps_min: 20 }),
        10,
    );
    let mut engine = FsmEngine::new();
    let id = engine.register(p);

    let t = "PLTR";
    // Trigger congress trade (two events within 14 days)
    engine.on_event(t, &Event::CongressTradeEvent { ticker: t.into(), ts: 0, amount_max: 100_000.0, member: "Pelosi".into() });
    engine.on_event(t, &Event::CongressTradeEvent { ticker: t.into(), ts: 3 * SECS, amount_max: 200_000.0, member: "Pelosi".into() });

    // Trigger squeeze within 10 bars
    let m = engine.on_event(t, &Event::Squeeze { ticker: t.into(), ts: 5 * SECS, si_pct: 18.0, ctb_bps: 60 });
    assert!(m.iter().any(|x| x.pattern_id == id), "Then(CongressTrade, FloatSqueeze) should fire");
}
