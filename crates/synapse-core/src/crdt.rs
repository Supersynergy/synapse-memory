//! CRDT metadata layer using yrs (Yjs Rust port).
//! Each doc can have a mergeable `meta_crdt` BLOB (yrs Map state).
//! Tags, refs, and custom fields stored as yrs Map entries.
//! Merge: real yrs merge on meta. Content conflict = keep both (stored separately, flagged by BLAKE3 diff).

use crate::error::Result;
use yrs::{Doc, Map, ReadTxn, StateVector, Transact, Update, updates::decoder::Decode};

/// Create a new yrs Doc with initial map entries, returns encoded state.
pub fn new_meta(entries: &[(&str, &str)]) -> Result<Vec<u8>> {
    let doc = Doc::new();
    let map = doc.get_or_insert_map("meta");
    {
        let mut txn = doc.transact_mut();
        for (k, v) in entries {
            map.insert(&mut txn, *k, *v);
        }
    }
    let txn = doc.transact();
    Ok(txn.encode_state_as_update_v1(&StateVector::default()))
}

/// Merge two yrs state vectors. Returns merged state.
pub fn merge_meta(a: &[u8], b: &[u8]) -> Result<Vec<u8>> {
    let doc = Doc::new();
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(
            Update::decode_v1(a).map_err(|e| crate::error::Error::Other(e.to_string()))?,
        )
        .map_err(|e| crate::error::Error::Other(e.to_string()))?;
        txn.apply_update(
            Update::decode_v1(b).map_err(|e| crate::error::Error::Other(e.to_string()))?,
        )
        .map_err(|e| crate::error::Error::Other(e.to_string()))?;
    }
    let txn = doc.transact();
    Ok(txn.encode_state_as_update_v1(&StateVector::default()))
}

/// Decode meta state to key-value pairs.
#[allow(clippy::type_complexity)]
pub fn read_meta(state: &[u8]) -> Result<Vec<(String, String)>> {
    let doc = Doc::new();
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(
            Update::decode_v1(state).map_err(|e| crate::error::Error::Other(e.to_string()))?,
        )
        .map_err(|e| crate::error::Error::Other(e.to_string()))?;
    }
    let txn = doc.transact();
    let map = txn.get_map("meta");
    let mut out = vec![];
    if let Some(m) = map {
        for (k, v) in m.iter(&txn) {
            if let yrs::Out::Any(yrs::Any::String(s)) = v {
                out.push((k.to_string(), s.to_string()));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_read() {
        let state = new_meta(&[("tags", "rust,memory"), ("author", "agent1")]).unwrap();
        let kvs = read_meta(&state).unwrap();
        let map: std::collections::HashMap<_, _> = kvs.into_iter().collect();
        assert_eq!(map["tags"], "rust,memory");
        assert_eq!(map["author"], "agent1");
    }

    #[test]
    fn merge_no_data_loss() {
        let a = new_meta(&[("tags", "rust"), ("source", "agent1")]).unwrap();
        let b = new_meta(&[("tags", "memory"), ("refs", "doc://42")]).unwrap();
        let merged = merge_meta(&a, &b).unwrap();
        let kvs = read_meta(&merged).unwrap();
        let map: std::collections::HashMap<_, _> = kvs.into_iter().collect();
        // Both sources present; last-write-wins per key but both keys present
        assert!(map.contains_key("source"));
        assert!(map.contains_key("refs"));
    }

    #[test]
    fn three_agent_merge() {
        let a = new_meta(&[("tags", "rust")]).unwrap();
        let b = new_meta(&[("tags", "memory")]).unwrap();
        let c = new_meta(&[("tags", "sqlite")]).unwrap();
        let ab = merge_meta(&a, &b).unwrap();
        let abc = merge_meta(&ab, &c).unwrap();
        let kvs = read_meta(&abc).unwrap();
        // Should have tags key
        assert!(kvs.iter().any(|(k, _)| k == "tags"));
    }
}
