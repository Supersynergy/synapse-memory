use synapse_market::stream::{LiveTick, TickStream};

struct MockStream {
    ticks: std::collections::VecDeque<LiveTick>,
}

impl MockStream {
    fn new(n: usize) -> Self {
        let ticks = (0..n)
            .map(|i| LiveTick {
                ts: i as i64 * 60,
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
async fn ingest_stream_1000_ticks_series_has_rows() {
    let mkt = synapse_market::Market::open_in_memory().unwrap();
    let stream = MockStream::new(1000);
    let n = mkt.ingest_stream("WIRE", stream, None).await.unwrap();
    assert_eq!(n, 1000);
    let count: i64 = mkt
        .conn
        .query_row("SELECT COUNT(*) FROM ohlcv_WIRE", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1000);
}

#[tokio::test]
async fn ingest_stream_max_ticks_respected() {
    let mkt = synapse_market::Market::open_in_memory().unwrap();
    let stream = MockStream::new(2000);
    let n = mkt.ingest_stream("WMAX", stream, Some(500)).await.unwrap();
    assert_eq!(n, 500);
}
