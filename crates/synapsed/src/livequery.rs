//! LiveQuery — WebSocket subscriptions to record changes.
//!
//! Pattern (mining-first, NOT 1:1 SurrealDB copy):
//!   - tokio::sync::broadcast MPMC lock-free channel for fanout
//!   - axum::extract::ws::WebSocket for subscribe path /live
//!   - filter via simple text-match on title/uri (compiled regex per sub)
//!   - lagged-receiver-drop policy (slow subscribers don't block fast ones)
//!
//! Design choices vs SurrealDB LIVE SELECT:
//!   - We don't intercept SQL: events emitted on Put/PutBatch op directly
//!   - Filter is text-substring (lowercase) — fast, no SQL parser
//!   - Channel cap 256 (drops oldest if subscriber lags)
//!   - Single port :9091 (separate from metrics :9090)

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveEvent {
    pub op: String, // "Put" | "PutBatch" | "Merge"
    pub id: i64,
    pub title: Option<String>,
    pub uri: Option<String>,
    pub ts: i64,
}

#[derive(Debug, Deserialize)]
struct SubscribeFilter {
    /// Substring match on title (lowercase). None = all.
    contains: Option<String>,
    /// URI prefix match. None = all.
    uri_prefix: Option<String>,
}

#[derive(Clone)]
pub struct LiveBroker {
    tx: broadcast::Sender<LiveEvent>,
}

impl LiveBroker {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Emit event — non-blocking. Lagged subscribers see Lagged err on next recv.
    pub fn emit(&self, ev: LiveEvent) {
        // Best-effort send; receivers count includes recently-dropped.
        let _ = self.tx.send(ev);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        self.tx.subscribe()
    }

    #[allow(dead_code)]
    pub fn n_subs(&self) -> usize {
        self.tx.receiver_count()
    }
}

pub async fn serve(broker: LiveBroker, addr: SocketAddr) {
    let state = Arc::new(broker);
    let app = Router::new()
        .route("/live", get(ws_handler))
        .with_state(state);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("livequery bind {addr}: {e}");
            return;
        }
    };
    info!("livequery on ws://{addr}/live");
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("livequery server: {e}");
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(broker): State<Arc<LiveBroker>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, broker))
}

async fn handle_socket(mut socket: WebSocket, broker: Arc<LiveBroker>) {
    // First message = subscription filter (JSON).
    let filter: SubscribeFilter = match socket.recv().await {
        Some(Ok(Message::Text(t))) => serde_json::from_str(t.as_str()).unwrap_or(SubscribeFilter {
            contains: None,
            uri_prefix: None,
        }),
        _ => SubscribeFilter {
            contains: None,
            uri_prefix: None,
        },
    };
    let mut rx = broker.subscribe();
    info!("ws subscriber attached, filter={:?}", filter);
    loop {
        tokio::select! {
            res = rx.recv() => {
                match res {
                    Ok(ev) => {
                        if !matches_filter(&ev, &filter) { continue; }
                        let json = match serde_json::to_string(&ev) {
                            Ok(j) => j,
                            Err(_) => continue,
                        };
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break; // client disconnected
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("ws subscriber lagged, dropped {n} events");
                        // continue — don't disconnect
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            client_msg = socket.recv() => {
                match client_msg {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    _ => {} // ignore other client messages (heartbeat etc)
                }
            }
        }
    }
    info!("ws subscriber detached");
}

fn matches_filter(ev: &LiveEvent, f: &SubscribeFilter) -> bool {
    if let Some(c) = &f.contains {
        let cl = c.to_lowercase();
        let title_match = ev
            .title
            .as_deref()
            .map(|t| t.to_lowercase().contains(&cl))
            .unwrap_or(false);
        if !title_match {
            return false;
        }
    }
    if let Some(p) = &f.uri_prefix {
        let prefix_match = ev.uri.as_deref().map(|u| u.starts_with(p)).unwrap_or(false);
        if !prefix_match {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_passes_all_when_empty() {
        let ev = LiveEvent {
            op: "Put".into(),
            id: 1,
            title: Some("hello".into()),
            uri: None,
            ts: 0,
        };
        let f = SubscribeFilter {
            contains: None,
            uri_prefix: None,
        };
        assert!(matches_filter(&ev, &f));
    }

    #[test]
    fn filter_contains_matches_substring() {
        let ev = LiveEvent {
            op: "Put".into(),
            id: 1,
            title: Some("Hello World".into()),
            uri: None,
            ts: 0,
        };
        let f = SubscribeFilter {
            contains: Some("WORLD".into()),
            uri_prefix: None,
        };
        assert!(matches_filter(&ev, &f));
    }

    #[test]
    fn filter_rejects_mismatch() {
        let ev = LiveEvent {
            op: "Put".into(),
            id: 1,
            title: Some("hello".into()),
            uri: None,
            ts: 0,
        };
        let f = SubscribeFilter {
            contains: Some("xyz".into()),
            uri_prefix: None,
        };
        assert!(!matches_filter(&ev, &f));
    }

    #[tokio::test]
    async fn broker_fanout() {
        let b = LiveBroker::new(16);
        let mut r1 = b.subscribe();
        let mut r2 = b.subscribe();
        b.emit(LiveEvent {
            op: "Put".into(),
            id: 7,
            title: None,
            uri: None,
            ts: 0,
        });
        let e1 = r1.recv().await.unwrap();
        let e2 = r2.recv().await.unwrap();
        assert_eq!(e1.id, 7);
        assert_eq!(e2.id, 7);
    }
}
