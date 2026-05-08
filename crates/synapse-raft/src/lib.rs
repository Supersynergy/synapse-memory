//! synapse-raft — raft consensus layer for replicated Synapse Store.
//!
//! Cluster-C of synapse-gap-sprint. Closes "Replication" gap (was 🟡, target 🟢).
//! Reference: `repos/hiqlite/` (raft+SQLite already fused, Apache 2.0).
//! Engine choice: `openraft` (databendlabs, 486 ghgrep hits).
//!
//! **STATUS**: trait-only scaffold. Wire openraft `RaftStorage` next iteration.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Single mutation logged to raft log + applied to state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogEntry {
    Put { key: String, value: Vec<u8> },
    Delete { key: String },
    Sql { stmt: String },
}

#[derive(Debug, thiserror::Error)]
pub enum RaftError {
    #[error("not enabled — build with --features openraft-backend")]
    NotEnabled,
    #[error("not leader, current leader: {0:?}")]
    NotLeader(Option<u64>),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage: {0}")]
    Storage(String),
}

#[async_trait]
pub trait RaftNode: Send + Sync {
    /// Append entry to raft log + replicate. Returns when committed.
    async fn submit(&self, entry: LogEntry) -> Result<u64, RaftError>;

    /// Current node id.
    fn node_id(&self) -> u64;

    /// True if this node is leader.
    async fn is_leader(&self) -> bool;
}

pub struct StubNode {
    pub id: u64,
}

#[async_trait]
impl RaftNode for StubNode {
    async fn submit(&self, _entry: LogEntry) -> Result<u64, RaftError> {
        Err(RaftError::NotEnabled)
    }
    fn node_id(&self) -> u64 {
        self.id
    }
    async fn is_leader(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stub_returns_not_enabled() {
        let n = StubNode { id: 1 };
        assert_eq!(n.node_id(), 1);
        assert!(!n.is_leader().await);
        let e = LogEntry::Put { key: "k".into(), value: vec![1, 2, 3] };
        assert!(matches!(n.submit(e).await, Err(RaftError::NotEnabled)));
    }

    #[test]
    fn log_entry_serde_roundtrip() {
        let e = LogEntry::Sql { stmt: "SELECT 1".into() };
        let bytes = serde_json::to_vec(&e).unwrap();
        let back: LogEntry = serde_json::from_slice(&bytes).unwrap();
        match back {
            LogEntry::Sql { stmt } => assert_eq!(stmt, "SELECT 1"),
            _ => panic!("variant mismatch"),
        }
    }
}
