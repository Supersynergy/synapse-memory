use synapse_market::pattern::{Pattern, Event};
use synapse_market::pattern::fsm::FsmEngine;
use synapse_market::store::page::Bar;

const SECS: i64 = 86_400;

fn bar(ts: i64, volume: f32) -> Bar {
    Bar { ts, open: 100.0, high: 101.0, low: 99.0, close: 100.5, volume }
}

#[test]
fn test_drought_buy() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::DroughtBuy { quiet_days: 30, min_value: 100_000.0 });

    // First buy — no match yet (no prior reference)
    let ev1 = Event::InsiderBuy { value_usd: 200_000.0, ts: 0, ticker: "AAPL".into() };
    let m1 = engine.on_event("AAPL", &ev1);
    assert!(m1.is_empty(), "no prior buy — should not match");

    // Second buy 31 days later — should match
    let ev2 = Event::InsiderBuy { value_usd: 250_000.0, ts: 31 * SECS, ticker: "AAPL".into() };
    let m2 = engine.on_event("AAPL", &ev2);
    assert_eq!(m2.len(), 1);
    assert_eq!(m2[0].pattern_id, id);
}

#[test]
fn test_drought_buy_too_soon() {
    let mut engine = FsmEngine::new();
    engine.register(Pattern::DroughtBuy { quiet_days: 30, min_value: 100_000.0 });

    let ev1 = Event::InsiderBuy { value_usd: 200_000.0, ts: 0, ticker: "AAPL".into() };
    engine.on_event("AAPL", &ev1);

    // Only 10 days later — should NOT match
    let ev2 = Event::InsiderBuy { value_usd: 250_000.0, ts: 10 * SECS, ticker: "AAPL".into() };
    let m = engine.on_event("AAPL", &ev2);
    assert!(m.is_empty());
}

#[test]
fn test_insider_cluster() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::InsiderCluster { k: 3, window_days: 14 });

    let ticker = "TSLA";
    // 3 buys within 14 days
    for i in 0..2i64 {
        let m = engine.on_event(ticker, &Event::InsiderBuy { value_usd: 50_000.0, ts: i * SECS, ticker: ticker.into() });
        assert!(m.is_empty(), "not enough buys yet");
    }
    let m = engine.on_event(ticker, &Event::InsiderBuy { value_usd: 50_000.0, ts: 5 * SECS, ticker: ticker.into() });
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].pattern_id, id);
}

#[test]
fn test_fda_triple() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::FdaTriple { window_days: 60 });

    let t = "MRNA";
    engine.on_event(t, &Event::Filing { kind: "8-K".into(), ticker: t.into(), ts: 0 });
    engine.on_event(t, &Event::Filing { kind: "S-3".into(), ticker: t.into(), ts: 10 * SECS });
    let m = engine.on_event(t, &Event::Filing { kind: "Phase-III".into(), ticker: t.into(), ts: 20 * SECS });
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].pattern_id, id);
}

#[test]
fn test_volume_spike() {
    let mut engine = FsmEngine::new();
    let id = engine.register(Pattern::VolumeSpike { multiplier: 2.0, window_bars: 5 });

    let t = "SPY";
    // Feed 5 normal bars (vol=100)
    for i in 0..5i64 {
        let m = engine.on_event(t, &Event::Candle(bar(i * 60, 100.0)));
        assert!(m.is_empty());
    }
    // Now spike bar vol=250 (> 2× mean 100)
    let m = engine.on_event(t, &Event::Candle(bar(6 * 60, 250.0)));
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].pattern_id, id);
}

#[test]
fn test_and_composition() {
    let p = Pattern::And(
        Box::new(Pattern::InsiderCluster { k: 2, window_days: 7 }),
        Box::new(Pattern::VolumeSpike { multiplier: 1.5, window_bars: 3 }),
    );
    let mut engine = FsmEngine::new();
    let id = engine.register(p);

    let t = "NVDA";
    // Satisfy cluster
    engine.on_event(t, &Event::InsiderBuy { value_usd: 100_000.0, ts: 0, ticker: t.into() });
    engine.on_event(t, &Event::InsiderBuy { value_usd: 100_000.0, ts: SECS, ticker: t.into() });

    // Satisfy volume spike — fill ring first
    engine.on_event(t, &Event::Candle(bar(100, 100.0)));
    engine.on_event(t, &Event::Candle(bar(200, 100.0)));
    engine.on_event(t, &Event::Candle(bar(300, 100.0)));
    let m = engine.on_event(t, &Event::Candle(bar(400, 300.0)));
    assert!(m.iter().any(|x| x.pattern_id == id), "And should fire");
}

#[test]
fn test_then_composition() {
    // InsiderCluster(k=2, w=7) THEN VolumeSpike(2.0, 3) within 5 bars
    let p = Pattern::Then(
        Box::new(Pattern::InsiderCluster { k: 2, window_days: 7 }),
        Box::new(Pattern::VolumeSpike { multiplier: 2.0, window_bars: 3 }),
        5,
    );
    let mut engine = FsmEngine::new();
    let id = engine.register(p);

    let t = "AMD";
    // Trigger A
    engine.on_event(t, &Event::InsiderBuy { value_usd: 50_000.0, ts: 0, ticker: t.into() });
    engine.on_event(t, &Event::InsiderBuy { value_usd: 50_000.0, ts: SECS, ticker: t.into() });

    // Fill volume ring
    engine.on_event(t, &Event::Candle(bar(1, 100.0)));
    engine.on_event(t, &Event::Candle(bar(2, 100.0)));
    engine.on_event(t, &Event::Candle(bar(3, 100.0)));
    // Spike — should trigger Then
    let m = engine.on_event(t, &Event::Candle(bar(4, 300.0)));
    assert!(m.iter().any(|x| x.pattern_id == id), "Then should fire");
}
