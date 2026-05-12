use synapse_market::filter::Bloom;
use xxhash_rust::xxh3::xxh3_64;

#[test]
fn insert_and_query_present() {
    let mut b = Bloom::new();
    for i in 0u64..10_000 {
        b.add(xxh3_64(&i.to_le_bytes()));
    }
    for i in 0u64..10_000 {
        assert!(b.contains(xxh3_64(&i.to_le_bytes())), "false negative at {i}");
    }
}

#[test]
fn fpr_absent() {
    let mut b = Bloom::new();
    for i in 0u64..10_000 {
        b.add(xxh3_64(&i.to_le_bytes()));
    }
    let mut fp = 0usize;
    for i in 10_000u64..20_000 {
        if b.contains(xxh3_64(&i.to_le_bytes())) {
            fp += 1;
        }
    }
    let fpr = fp as f64 / 10_000.0;
    assert!(fpr <= 0.01, "FPR {fpr:.4} > 1%");
}

#[test]
fn roundtrip_serialize() {
    let mut b = Bloom::new();
    for i in 0u64..5_000 {
        b.add(xxh3_64(&i.to_le_bytes()));
    }
    let bytes = b.serialize();
    assert_eq!(bytes.len(), 16384);
    let b2 = Bloom::deserialize(&bytes).expect("deserialize failed");
    for i in 0u64..5_000 {
        assert!(b2.contains(xxh3_64(&i.to_le_bytes())));
    }
}

#[test]
fn deserialize_wrong_len_returns_none() {
    assert!(Bloom::deserialize(&[0u8; 100]).is_none());
}
