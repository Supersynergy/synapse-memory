//! Multi-writer replication — Phase 3.
//!
//! Design: each writer journals operations as CRDT frames. Peers exchange
//! their `CRDTOpsLog` chunks, merge deterministically, and optionally compact
//! into a new root manifest. v0.2 wires an Automerge backend behind the
//! `crdt` feature in `automerge_wire`; the LWW merge here remains the
//! zero-dep baseline and is always available.

pub mod automerge_wire;

use crate::error::Result;
use serde::{Deserialize, Serialize};

/// Opaque operation identifier — a content-addressed hash of the op body.
pub type OpId = [u8; 32];

/// One replicated write to a Synapse store.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Op {
    Put {
        doc_id: String,
        blob_hash: [u8; 32],
        ts: i64,
    },
    Delete {
        doc_id: String,
        ts: i64,
    },
    RenameTitle {
        doc_id: String,
        new_title: String,
        ts: i64,
    },
}

/// Replication transport trait.
pub trait SyncPeer {
    fn peer_id(&self) -> &str;
    fn pull_since(&mut self, op_id: OpId) -> Result<Vec<(OpId, Op)>>;
    fn push(&mut self, ops: &[(OpId, Op)]) -> Result<()>;
}

/// Deterministic merge: last-writer-wins on `ts`, tie-break by `OpId` lex order.
///
/// Enough for Phase 3 Put/Delete/Rename; richer CRDTs (RGA for text edits) come in v0.4.
pub fn merge_lww(local: &[(OpId, Op)], remote: &[(OpId, Op)]) -> Vec<(OpId, Op)> {
    use std::collections::HashMap;

    fn key(op: &Op) -> &str {
        match op {
            Op::Put { doc_id, .. } => doc_id,
            Op::Delete { doc_id, .. } => doc_id,
            Op::RenameTitle { doc_id, .. } => doc_id,
        }
    }
    fn ts(op: &Op) -> i64 {
        match op {
            Op::Put { ts, .. } | Op::Delete { ts, .. } | Op::RenameTitle { ts, .. } => *ts,
        }
    }

    let mut winners: HashMap<String, (OpId, Op)> = HashMap::new();
    for (oid, op) in local.iter().chain(remote.iter()).cloned() {
        let k = key(&op).to_string();
        match winners.get(&k) {
            None => {
                winners.insert(k, (oid, op));
            }
            Some((cur_oid, cur_op)) => {
                let keep_cur = ts(cur_op) > ts(&op) || (ts(cur_op) == ts(&op) && cur_oid > &oid);
                if !keep_cur {
                    winners.insert(k, (oid, op));
                }
            }
        }
    }
    let mut out: Vec<_> = winners.into_values().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lww_later_timestamp_wins() {
        let a = Op::Put {
            doc_id: "x".into(),
            blob_hash: [1; 32],
            ts: 100,
        };
        let b = Op::Put {
            doc_id: "x".into(),
            blob_hash: [2; 32],
            ts: 200,
        };
        let merged = merge_lww(&[([0; 32], a)], &[([1; 32], b)]);
        assert_eq!(merged.len(), 1);
        if let Op::Put { blob_hash, .. } = &merged[0].1 {
            assert_eq!(*blob_hash, [2; 32]);
        } else {
            panic!("expected Put");
        }
    }

    #[test]
    fn lww_delete_wins_over_earlier_put() {
        let put = Op::Put {
            doc_id: "y".into(),
            blob_hash: [1; 32],
            ts: 100,
        };
        let del = Op::Delete {
            doc_id: "y".into(),
            ts: 150,
        };
        let merged = merge_lww(&[([0; 32], put)], &[([1; 32], del)]);
        assert!(matches!(merged[0].1, Op::Delete { .. }));
    }
}
