//! LiveRelate — real-time graph delta broadcast.
//!
//! Pattern: tokio::sync::broadcast channel (Surreal-killer parity).
//! Every `relate()` call emits RelateEvent. Subscribers receive deltas.
//! Used by GraphQL/REST layer to push live updates to clients via SSE/WS.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelateEvent {
    pub seq: u64,
    pub op: EventOp,
    pub from_id: i64,
    pub to_id: i64,
    pub rel: String,
    pub weight: f64,
    pub ts_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventOp {
    Insert,
    Delete,
    Update,
}

/// LiveRelate — broadcast hub.
/// Capacity 1024 — older events dropped for slow subscribers (lag-tolerant).
pub struct LiveRelate {
    seq: AtomicU64,
    sender: Arc<tokio::sync::broadcast::Sender<RelateEvent>>,
}

impl LiveRelate {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = tokio::sync::broadcast::channel(capacity);
        Self {
            seq: AtomicU64::new(0),
            sender: Arc::new(tx),
        }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RelateEvent> {
        self.sender.subscribe()
    }

    pub fn emit(&self, op: EventOp, from_id: i64, to_id: i64, rel: &str, weight: f64) {
        let event = RelateEvent {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            op,
            from_id,
            to_id,
            rel: rel.to_string(),
            weight,
            ts_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let _ = self.sender.send(event); // ignore send-err if no subscribers
    }

    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for LiveRelate {
    fn default() -> Self {
        Self::new(1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emit_and_receive() {
        let live = LiveRelate::default();
        let mut rx = live.subscribe();
        live.emit(EventOp::Insert, 1, 2, "REL", 1.0);
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.from_id, 1);
        assert_eq!(ev.to_id, 2);
        assert_eq!(ev.seq, 0);
        assert!(matches!(ev.op, EventOp::Insert));
    }

    #[tokio::test]
    async fn seq_increments() {
        let live = LiveRelate::default();
        let mut rx = live.subscribe();
        live.emit(EventOp::Insert, 1, 2, "R", 1.0);
        live.emit(EventOp::Update, 1, 2, "R", 2.0);
        assert_eq!(rx.recv().await.unwrap().seq, 0);
        assert_eq!(rx.recv().await.unwrap().seq, 1);
    }

    #[tokio::test]
    async fn no_subscriber_no_panic() {
        let live = LiveRelate::default();
        live.emit(EventOp::Delete, 5, 6, "R", 0.0);
        // no panic = pass
    }

    #[tokio::test]
    async fn multiple_subscribers_each_get_event() {
        let live = LiveRelate::default();
        let mut rx1 = live.subscribe();
        let mut rx2 = live.subscribe();
        live.emit(EventOp::Insert, 9, 10, "R", 0.5);
        assert_eq!(rx1.recv().await.unwrap().from_id, 9);
        assert_eq!(rx2.recv().await.unwrap().from_id, 9);
    }
}
