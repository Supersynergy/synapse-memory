pub mod parser;
pub mod ws;

pub use parser::Parser;
pub use ws::WebSocketTickStream;

/// A single live trade tick from a streaming feed.
#[derive(Debug, Clone)]
pub struct LiveTick {
    /// Unix timestamp (seconds).
    pub ts: i64,
    /// Trade price.
    pub price: f64,
    /// Trade quantity / size.
    pub qty: f64,
}

/// Async tick source. Implement for custom feeds (mock, file replay, etc.).
pub trait TickStream: Send {
    type Item: Send;
    /// Returns the next tick, or `None` when the stream is exhausted / closed.
    async fn next_tick(&mut self) -> Option<Self::Item>;
}
