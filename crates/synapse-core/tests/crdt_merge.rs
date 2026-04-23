use synapse_core::crdt::new_meta;
use synapse_core::{snap, PutRequest, Store};

fn put_with_crdt(store: &mut Store, uri: &str, text: &str, tags: &str) -> i64 {
    let crdt = new_meta(&[("tags", tags)]).unwrap();
    store
        .put_with_crdt(
            &PutRequest {
                uri: Some(uri.into()),
                text: text.into(),
                ..Default::default()
            },
            Some(crdt),
        )
        .unwrap()
}

#[test]
fn three_concurrent_writers_no_data_loss() {
    // Three writers produce brainpacks independently, then merge A+B+C
    let db_a = tempfile::NamedTempFile::new().unwrap();
    let db_b = tempfile::NamedTempFile::new().unwrap();
    let db_c = tempfile::NamedTempFile::new().unwrap();

    {
        let mut s = Store::open(db_a.path()).unwrap();
        put_with_crdt(&mut s, "doc://1", "shared doc", "rust");
        put_with_crdt(&mut s, "doc://2", "only in A", "agent1");
    }
    {
        let mut s = Store::open(db_b.path()).unwrap();
        put_with_crdt(&mut s, "doc://1", "shared doc", "memory");
        put_with_crdt(&mut s, "doc://3", "only in B", "agent2");
    }
    {
        let mut s = Store::open(db_c.path()).unwrap();
        put_with_crdt(&mut s, "doc://1", "shared doc", "sqlite");
        put_with_crdt(&mut s, "doc://4", "only in C", "agent3");
    }

    let pack_a = tempfile::NamedTempFile::new().unwrap();
    let pack_b = tempfile::NamedTempFile::new().unwrap();
    let pack_c = tempfile::NamedTempFile::new().unwrap();
    snap::export(db_a.path(), pack_a.path(), 3).unwrap();
    snap::export(db_b.path(), pack_b.path(), 3).unwrap();
    snap::export(db_c.path(), pack_c.path(), 3).unwrap();

    // Merge A+B
    let pack_ab = tempfile::NamedTempFile::new().unwrap();
    snap::merge_packs(pack_a.path(), pack_b.path(), pack_ab.path(), 3).unwrap();

    // Merge AB+C
    let pack_abc = tempfile::NamedTempFile::new().unwrap();
    snap::merge_packs(pack_ab.path(), pack_c.path(), pack_abc.path(), 3).unwrap();

    // Restore and verify
    let db_merged = tempfile::NamedTempFile::new().unwrap();
    snap::import(pack_abc.path(), db_merged.path()).unwrap();
    let merged = Store::open(db_merged.path()).unwrap();
    let stats = merged.stats().unwrap();

    // 4 unique docs total
    assert_eq!(stats.docs, 4, "expected 4 unique docs, got {}", stats.docs);

    // Check CRDT state on doc://1 — should have been merged
    let crdt_bytes: Option<Vec<u8>> = merged
        .conn
        .query_row(
            "SELECT meta_crdt FROM docs WHERE uri = 'doc://1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let crdt_bytes = crdt_bytes.expect("doc://1 should have meta_crdt");
    let kvs = synapse_core::crdt::read_meta(&crdt_bytes).unwrap();
    // At minimum, the merged tags key must be present
    assert!(
        kvs.iter().any(|(k, _)| k == "tags"),
        "merged crdt must have tags key"
    );
}

#[test]
fn extension_agnostic_roundtrip() {
    let db = tempfile::NamedTempFile::new().unwrap();
    {
        let mut s = Store::open(db.path()).unwrap();
        s.put(&PutRequest {
            text: "ext agnostic test".into(),
            ..Default::default()
        })
        .unwrap();
    }

    // Export as .syn
    let pack_syn = tempfile::Builder::new().suffix(".syn").tempfile().unwrap();
    snap::export(db.path(), pack_syn.path(), 3).unwrap();

    // Import using path with .brainpack extension (rename via symlink workaround: just pass the .syn path directly)
    let restored = tempfile::Builder::new()
        .suffix(".brainpack")
        .tempfile()
        .unwrap();
    snap::import(pack_syn.path(), restored.path()).unwrap();

    let s = Store::open(restored.path()).unwrap();
    assert_eq!(s.stats().unwrap().docs, 1);
}
