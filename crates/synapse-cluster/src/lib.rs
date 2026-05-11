//! `synapse-cluster` — multi-node AP cluster via TCP gossip + CRDT merge.
//!
//! Each [`Node`] owns a local `synapse-core` Store plus a list of peers.
//! A background [`Node::gossip_loop`] fires every `gossip_interval` ms,
//! pulls the delta since the peer's last-known clock, and applies it via
//! [`synapse_core::sync::merge_lww`] (or the Automerge backend when the
//! `crdt` feature is active on synapse-core).
//!
//! Wire protocol (length-prefix framing, JSON payload):
//! ```text
//! [4 bytes LE length][JSON Message bytes]
//! ```

pub mod proto;
pub mod transport;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use synapse_core::sync::{merge_lww, Op, OpId};

pub type NodeId = String;

/// Information about a remote peer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: NodeId,
    pub addr: SocketAddr,
}

/// Logical clock: monotonically increasing counter per node.
pub type Clock = u64;

/// Shared state protected by RwLock so gossip loop + server can both access.
struct NodeState {
    id: NodeId,
    /// Local op log: (clock, op_id, op).
    ops: Vec<(Clock, OpId, Op)>,
    /// Last clock we pulled from each peer.
    peer_clocks: HashMap<NodeId, Clock>,
    /// Current logical clock (incremented on each local put).
    clock: Clock,
}

/// A Synapse cluster node.
pub struct Node {
    pub id: NodeId,
    pub peers: Vec<PeerInfo>,
    /// Gossip interval.
    pub gossip_interval: Duration,
    pub(crate) state: Arc<RwLock<NodeState>>,
    listen_addr: SocketAddr,
}

impl Node {
    pub fn new(id: impl Into<String>, listen_addr: SocketAddr) -> Self {
        let id = id.into();
        let state = Arc::new(RwLock::new(NodeState {
            id: id.clone(),
            ops: Vec::new(),
            peer_clocks: HashMap::new(),
            clock: 0,
        }));
        Self {
            id,
            peers: Vec::new(),
            gossip_interval: Duration::from_millis(500),
            state,
            listen_addr,
        }
    }

    pub fn add_peer(&mut self, peer: PeerInfo) {
        self.peers.push(peer);
    }

    /// Write a local op, increment clock, store in log.
    pub async fn put_op(&self, op: Op) -> Result<OpId> {
        let op_id = op_id_for(&op);
        let mut st = self.state.write().await;
        st.clock += 1;
        let c = st.clock;
        st.ops.push((c, op_id, op));
        Ok(op_id)
    }

    /// Expose local ops for external inspection (e.g. tests).
    pub async fn local_ops(&self) -> Vec<(OpId, Op)> {
        let st = self.state.read().await;
        st.ops.iter().map(|(_, id, op)| (*id, op.clone())).collect()
    }

    /// Apply a delta arriving from a peer (CRDT merge into local log).
    pub async fn merge_peer_delta(&self, delta: Vec<(OpId, Op)>) {
        let mut st = self.state.write().await;
        let local: Vec<(OpId, Op)> = st.ops.iter().map(|(_, id, op)| (*id, op.clone())).collect();
        let merged = merge_lww(&local, &delta);
        // Rebuild op log from merged set; preserve clock ordering for local ops.
        let existing_clock: HashMap<OpId, Clock> =
            st.ops.iter().map(|(c, id, _)| (*id, *c)).collect();
        st.ops = merged
            .into_iter()
            .map(|(id, op)| {
                let c = existing_clock.get(&id).copied().unwrap_or({
                    st.clock += 1;
                    st.clock
                });
                (c, id, op)
            })
            .collect();
        st.ops.sort_by_key(|(c, _, _)| *c);
    }

    /// Start the TCP server that responds to gossip pull/push requests.
    /// Returns a JoinHandle — caller must `.await` or abort on shutdown.
    pub async fn start_server(
        node: Arc<Self>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let listener = TcpListener::bind(node.listen_addr).await?;
        info!(id = %node.id, addr = %node.listen_addr, "cluster server listening");
        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer_addr)) => {
                        debug!(peer_addr = %peer_addr, "incoming connection");
                        let node = node.clone();
                        tokio::spawn(async move {
                            if let Err(e) = transport::handle_connection(stream, node).await {
                                warn!("connection error: {e}");
                            }
                        });
                    }
                    Err(e) => warn!("accept error: {e}"),
                }
            }
        });
        Ok(handle)
    }

    /// Periodic gossip loop: for each peer, pull changes since last-known clock.
    pub async fn gossip_loop(node: Arc<Self>) {
        loop {
            tokio::time::sleep(node.gossip_interval).await;
            let peers = node.peers.clone();
            for peer in &peers {
                let since = {
                    let st = node.state.read().await;
                    st.peer_clocks.get(&peer.id).copied().unwrap_or(0)
                };
                match transport::pull_from_peer(peer, since).await {
                    Ok(delta) if !delta.is_empty() => {
                        let new_clock = delta.len() as Clock + since;
                        node.merge_peer_delta(delta).await;
                        let mut st = node.state.write().await;
                        st.peer_clocks.insert(peer.id.clone(), new_clock);
                        debug!(peer = %peer.id, "merged delta");
                    }
                    Ok(_) => {}
                    Err(e) => warn!(peer = %peer.id, "gossip pull failed: {e}"),
                }
            }
        }
    }
}

fn op_id_for(op: &Op) -> OpId {
    let bytes = serde_json::to_vec(op).unwrap_or_default();
    let h = blake3_hash(&bytes);
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use synapse_core::sync::Op;

    /// 2-node smoke test: put on node1, gossip tick, node2 sees doc.
    #[tokio::test]
    async fn two_node_gossip_smoke() {
        let addr1: SocketAddr = "127.0.0.1:19901".parse().unwrap();
        let addr2: SocketAddr = "127.0.0.1:19902".parse().unwrap();

        let mut node1 = Node::new("node1", addr1);
        let node2 = Arc::new(Node::new("node2", addr2));

        // node1 knows about node2
        node1.add_peer(PeerInfo { id: "node2".into(), addr: addr2 });
        let node1 = Arc::new(node1);

        // Start servers
        let _s1 = Node::start_server(node1.clone()).await.unwrap();
        let _s2 = Node::start_server(node2.clone()).await.unwrap();

        // Give servers a moment to bind
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Put a doc on node1
        let op = Op::Put {
            doc_id: "hello-world".into(),
            blob_hash: [42u8; 32],
            ts: 1000,
        };
        let t0 = std::time::Instant::now();
        node1.put_op(op).await.unwrap();

        // Single gossip tick: node1 pushes to node2
        let ops = node1.local_ops().await;
        transport::push_to_peer(
            &PeerInfo { id: "node2".into(), addr: addr2 },
            ops,
        ).await.unwrap();

        let elapsed = t0.elapsed();

        // node2 should now have the doc
        let n2_ops = node2.local_ops().await;
        assert_eq!(n2_ops.len(), 1, "node2 must have the propagated op");
        match &n2_ops[0].1 {
            Op::Put { doc_id, blob_hash, .. } => {
                assert_eq!(doc_id, "hello-world");
                assert_eq!(blob_hash, &[42u8; 32]);
            }
            _ => panic!("expected Put op"),
        }

        // Latency assertion: push should complete in <200ms on loopback
        assert!(elapsed < Duration::from_millis(200),
            "gossip push took {:?}, expected <200ms", elapsed);
    }

    #[tokio::test]
    async fn crdt_merge_idempotent() {
        let node = Node::new("solo", "127.0.0.1:0".parse().unwrap());
        let op = Op::Put { doc_id: "x".into(), blob_hash: [1; 32], ts: 100 };
        node.put_op(op.clone()).await.unwrap();

        // Merging same op twice must not duplicate
        let ops = node.local_ops().await;
        node.merge_peer_delta(ops.clone()).await;
        node.merge_peer_delta(ops).await;

        let final_ops = node.local_ops().await;
        assert_eq!(final_ops.len(), 1);
    }
}

fn blake3_hash(data: &[u8]) -> [u8; 32] {
    // Use a simple deterministic hash; blake3 dep lives in synapse-core.
    // We avoid pulling blake3 directly — fold bytes into 32 bytes via XOR+rotate.
    let mut out = [0u8; 32];
    for (i, b) in data.iter().enumerate() {
        out[i % 32] ^= b.wrapping_add((i >> 5) as u8);
    }
    out
}
