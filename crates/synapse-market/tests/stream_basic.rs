use synapse_market::stream::{LiveTick, Parser, TickStream};

/// Mock stream backed by a pre-filled Vec.
struct MockStream {
    ticks: std::collections::VecDeque<LiveTick>,
}

impl MockStream {
    fn new(n: usize) -> Self {
        let ticks = (0..n)
            .map(|i| LiveTick {
                ts: i as i64,
                price: 100.0 + i as f64 * 0.01,
                qty: 1.0,
            })
            .collect();
        Self { ticks }
    }
}

impl TickStream for MockStream {
    type Item = LiveTick;
    async fn next_tick(&mut self) -> Option<Self::Item> {
        self.ticks.pop_front()
    }
}

#[tokio::test]
async fn ingest_10k_ticks() {
    let mkt = synapse_market::Market::open_in_memory().unwrap();
    let stream = MockStream::new(10_000);
    let n = mkt.ingest_stream("TEST", stream, None).await.unwrap();
    assert_eq!(n, 10_000);

    // Verify rows landed in ohlcv table
    let count: i64 = mkt
        .conn
        .query_row("SELECT COUNT(*) FROM ohlcv_TEST", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 10_000);
}

#[tokio::test]
async fn max_ticks_cap() {
    let mkt = synapse_market::Market::open_in_memory().unwrap();
    let stream = MockStream::new(5_000);
    let n = mkt.ingest_stream("CAP", stream, Some(200)).await.unwrap();
    assert_eq!(n, 200);
}

#[tokio::test]
async fn backpressure_empty_stream() {
    let mkt = synapse_market::Market::open_in_memory().unwrap();
    let stream = MockStream::new(0);
    let n = mkt.ingest_stream("EMPTY", stream, None).await.unwrap();
    assert_eq!(n, 0);
}

// --- Parser tests ---

#[test]
fn parser_generic_three_samples() {
    let samples = [
        r#"{"ts":1,"ticker":"A","price":1.0,"qty":1.0}"#,
        r#"{"ts":2,"ticker":"B","price":2.5,"qty":10.0,"side":"sell"}"#,
        r#"{"ts":3,"price":99.99,"qty":0.5}"#,
    ];
    for s in &samples {
        assert!(Parser::Generic.parse(s).is_some(), "failed: {s}");
    }
}

#[test]
fn parser_polygon_three_samples() {
    let samples = [
        r#"[{"ev":"T","sym":"AAPL","t":1700000000000,"p":150.0,"s":50}]"#,
        r#"[{"ev":"T","sym":"MSFT","t":1700000001000,"p":300.0,"s":10}]"#,
        r#"[{"ev":"T","sym":"TSLA","t":1700000002000,"p":200.0,"s":25}]"#,
    ];
    for s in &samples {
        assert!(Parser::PolygonV3.parse(s).is_some(), "failed: {s}");
    }
    // Non-trade skipped
    assert!(
        Parser::PolygonV3
            .parse(r#"[{"ev":"Q","sym":"X","t":1,"p":1.0,"s":1}]"#)
            .is_none()
    );
}

#[test]
fn parser_tradier_three_samples() {
    let samples = [
        r#"{"type":"trade","symbol":"AAPL","price":150.0,"size":100,"timestamp":1000}"#,
        r#"{"type":"trade","symbol":"MSFT","price":300.0,"size":200,"timestamp":2000}"#,
        r#"{"type":"trade","symbol":"TSLA","price":200.0,"size":50,"timestamp":3000}"#,
    ];
    for s in &samples {
        assert!(Parser::TradierV1.parse(s).is_some(), "failed: {s}");
    }
    assert!(
        Parser::TradierV1
            .parse(r#"{"type":"quote","symbol":"X","price":1.0,"size":1,"timestamp":1}"#)
            .is_none()
    );
}
