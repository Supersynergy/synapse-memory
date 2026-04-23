//! Automerge-backed CRDT wire format for multi-writer sync.
//!
//! Encoding strategy: we keep a single persistent Automerge document per
//! replica. Ops are stored as scalar-byte entries directly on the root map,
//! keyed by their content-hash `OpId` (hex-encoded). Because both peers write
//! to the same root object by id, merges union trivially and deterministically.
//!
//! Feature-gated on `crdt`.

#[cfg(feature = "crdt")]
pub use imp::*;
#[cfg(not(feature = "crdt"))]
pub use stub::*;

#[cfg(feature = "crdt")]
mod imp {
    use super::super::{Op, OpId};
    use crate::error::{Error, Result};
    use automerge::transaction::Transactable;
    use automerge::{AutoCommit, ReadDoc, ROOT};

    /// Apply a batch of ops onto an existing Automerge document.
    pub fn apply_ops(doc: &mut AutoCommit, ops: &[(OpId, Op)]) -> Result<()> {
        for (oid, op) in ops {
            let key = hex::encode(oid);
            let body = serde_json::to_vec(op).map_err(Error::from)?;
            doc.put(ROOT, key.as_str(), body)
                .map_err(|e| Error::Format(format!("automerge put: {e}")))?;
        }
        Ok(())
    }

    /// Encode a fresh Automerge document containing just `ops`.
    pub fn encode_ops(ops: &[(OpId, Op)]) -> Result<Vec<u8>> {
        let mut doc = AutoCommit::new();
        apply_ops(&mut doc, ops)?;
        Ok(doc.save())
    }

    /// Decode + merge a remote Automerge payload into the local op set and
    /// return the union in deterministic order.
    pub fn merge_payload(local: &[(OpId, Op)], remote: &[u8]) -> Result<Vec<(OpId, Op)>> {
        let mut doc = AutoCommit::new();
        apply_ops(&mut doc, local)?;
        let mut incoming =
            AutoCommit::load(remote).map_err(|e| Error::Format(format!("automerge load: {e}")))?;
        doc.merge(&mut incoming)
            .map_err(|e| Error::Format(format!("automerge merge: {e}")))?;

        let mut out: Vec<(OpId, Op)> = Vec::new();
        for key in doc.keys(ROOT) {
            let val = match doc.get(ROOT, &key) {
                Ok(Some((v, _))) => v,
                _ => continue,
            };
            let bytes = match val {
                automerge::Value::Scalar(s) => match s.as_ref() {
                    automerge::ScalarValue::Bytes(b) => b.clone(),
                    _ => continue,
                },
                _ => continue,
            };
            let op: Op = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let mut oid = [0u8; 32];
            let decoded = match hex::decode(&key) {
                Ok(v) if v.len() == 32 => v,
                _ => continue,
            };
            oid.copy_from_slice(&decoded);
            out.push((oid, op));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    #[cfg(test)]
    mod tests {
        use super::super::super::{Op, OpId};
        use super::*;

        fn fake_id(tag: u8) -> OpId {
            let mut a = [0u8; 32];
            a[0] = tag;
            a
        }

        #[test]
        fn encode_then_merge_is_union() {
            let a = (
                fake_id(1),
                Op::Put {
                    doc_id: "x".into(),
                    blob_hash: [9; 32],
                    ts: 100,
                },
            );
            let b = (
                fake_id(2),
                Op::Put {
                    doc_id: "y".into(),
                    blob_hash: [7; 32],
                    ts: 200,
                },
            );
            let payload = encode_ops(&[b.clone()]).unwrap();
            let merged = merge_payload(&[a.clone()], &payload).unwrap();
            assert_eq!(merged.len(), 2);
            let ids: Vec<_> = merged.iter().map(|x| x.0).collect();
            assert!(ids.contains(&fake_id(1)));
            assert!(ids.contains(&fake_id(2)));
        }

        #[test]
        fn merge_is_commutative() {
            let a = (
                fake_id(1),
                Op::Put {
                    doc_id: "x".into(),
                    blob_hash: [9; 32],
                    ts: 100,
                },
            );
            let b = (
                fake_id(2),
                Op::Delete {
                    doc_id: "y".into(),
                    ts: 200,
                },
            );
            let ab = merge_payload(&[a.clone()], &encode_ops(&[b.clone()]).unwrap()).unwrap();
            let ba = merge_payload(&[b.clone()], &encode_ops(&[a.clone()]).unwrap()).unwrap();
            assert_eq!(
                ab.iter().map(|x| x.0).collect::<Vec<_>>(),
                ba.iter().map(|x| x.0).collect::<Vec<_>>()
            );
        }
    }
}

#[cfg(not(feature = "crdt"))]
mod stub {
    use super::super::{Op, OpId};
    use crate::error::Result;

    pub fn encode_ops(_ops: &[(OpId, Op)]) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }

    pub fn merge_payload(local: &[(OpId, Op)], _remote: &[u8]) -> Result<Vec<(OpId, Op)>> {
        Ok(local.to_vec())
    }
}
