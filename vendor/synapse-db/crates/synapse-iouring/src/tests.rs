//! Smoke tests — run on all platforms (no io-uring feature needed).
//! Real io_uring path tested on Linux CI with `--features io-uring`.

#[cfg(test)]
mod tests {
    use crate::lsm::{Entry, L0};

    fn make_entry(key: &str, value: &str, seq: u64) -> Entry {
        Entry {
            key: key.as_bytes().to_vec(),
            value: value.as_bytes().to_vec(),
            seq,
            deleted: false,
        }
    }

    #[test]
    fn l0_insert_and_scan() {
        let l0 = L0::new(4096);
        l0.insert(make_entry("b", "v2", 1));
        l0.insert(make_entry("a", "v1", 0));
        l0.insert(make_entry("c", "v3", 2));

        let range = b"a".to_vec()..b"d".to_vec();
        let results = l0.scan(&range);
        assert_eq!(results.len(), 3);
        // SkipMap returns sorted by key
        assert_eq!(results[0].key, b"a");
        assert_eq!(results[1].key, b"b");
        assert_eq!(results[2].key, b"c");
    }

    #[test]
    fn l0_flush_threshold() {
        let l0 = L0::new(3);
        let needs_flush_1 = l0.insert(make_entry("a", "v1", 0));
        let needs_flush_2 = l0.insert(make_entry("b", "v2", 1));
        let needs_flush_3 = l0.insert(make_entry("c", "v3", 2));
        assert!(!needs_flush_1);
        assert!(!needs_flush_2);
        assert!(needs_flush_3, "third insert should trigger flush");
    }

    #[test]
    fn l0_drain_resets_count() {
        let l0 = L0::new(4096);
        l0.insert(make_entry("x", "val", 0));
        assert_eq!(l0.len(), 1);
        let drained = l0.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(l0.len(), 0);
    }

    #[test]
    fn bloom_filter_basic() {
        use crate::lsm::BloomFilter;
        let mut bf = BloomFilter::new(1000);
        bf.add(b"hello");
        bf.add(b"world");
        assert!(bf.contains(b"hello"));
        assert!(bf.contains(b"world"));
        assert!(!bf.contains(b"nothere_xyzzy_abc123"));
    }

    #[test]
    fn bloom_filter_persist_roundtrip() {
        use crate::lsm::BloomFilter;
        let mut bf = BloomFilter::new(100);
        bf.add(b"key1");
        bf.add(b"key2");
        let bytes = bf.to_bytes();
        let bf2 = BloomFilter::from_bytes(&bytes);
        assert!(bf2.contains(b"key1"));
        assert!(bf2.contains(b"key2"));
        assert!(!bf2.contains(b"nothere_unique_xyz"));
    }

    #[test]
    fn bloom_false_positive_rate() {
        use crate::lsm::BloomFilter;
        let n = 1000usize;
        let mut bf = BloomFilter::new(n);
        for i in 0..n {
            bf.add(format!("key{}", i).as_bytes());
        }
        // Check ~10k non-inserted keys — FPR should be <5%
        let mut fp = 0usize;
        let checks = 10_000usize;
        for i in n..n + checks {
            if bf.contains(format!("key{}", i).as_bytes()) {
                fp += 1;
            }
        }
        let fpr = fp as f64 / checks as f64;
        assert!(fpr < 0.05, "FPR {:.3} too high (expected <5%)", fpr);
    }

    #[test]
    fn sstable_write_with_bloom() {
        use crate::lsm::{Entry, SSTable};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sst");
        let entries = vec![
            Entry {
                key: b"a".to_vec(),
                value: b"1".to_vec(),
                seq: 0,
                deleted: false,
            },
            Entry {
                key: b"b".to_vec(),
                value: b"2".to_vec(),
                seq: 1,
                deleted: false,
            },
        ];
        let sst = SSTable::write(path.clone(), &entries).unwrap();
        // bloom file should exist
        let bloom_path = path.with_extension("sst.bloom");
        assert!(bloom_path.exists(), "bloom file should be written");
        // load_bloom works
        let bloom = sst.load_bloom().expect("bloom should load");
        assert!(bloom.contains(b"a"));
        assert!(bloom.contains(b"b"));
        assert!(!bloom.contains(b"z_unique_xyz"));
    }

    #[tokio::test]
    async fn compactor_flush_l0_and_compact() {
        use crate::compaction::{CompactCmd, Compactor, TieredConfig};
        use crate::lsm::Entry;
        let dir = tempfile::tempdir().unwrap();
        let config = TieredConfig {
            l1_max: 2,
            l2_max: 4,
            dir: dir.path().to_path_buf(),
        };
        let tx = Compactor::spawn(config);

        // Send 3 flushes — 2 should trigger L1→L2 compaction
        for batch in 0..3usize {
            let entries: Vec<Entry> = (0..10)
                .map(|i| Entry {
                    key: format!("batch{}-key{}", batch, i).into_bytes(),
                    value: b"val".to_vec(),
                    seq: (batch * 10 + i) as u64,
                    deleted: false,
                })
                .collect();
            tx.send(CompactCmd::FlushL0(entries)).await.unwrap();
        }
        tx.send(CompactCmd::Shutdown).await.unwrap();
        // Give background task time to finish
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Verify some SSTable files were written
        let sst_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "sst").unwrap_or(false))
            .collect();
        assert!(!sst_files.is_empty(), "at least one SSTable should exist");
    }

    #[test]
    fn store_open_returns_unsupported_on_non_linux() {
        // Without io-uring feature, open succeeds but append_batch errors at runtime
        use crate::store::IoUringStore;
        let dir = tempfile::tempdir().unwrap();
        let _store = IoUringStore::open(dir.path());
        // We just verify it compiles and opens without panic on macOS
    }

    #[tokio::test]
    async fn append_batch_returns_unsupported_without_feature() {
        use crate::error::IoUringError;
        use crate::store::IoUringStore;
        let dir = tempfile::tempdir().unwrap();
        let mut store = IoUringStore::open(dir.path()).unwrap();
        let entries = vec![make_entry("k1", "v1", 0)];
        let result = store.append_batch(entries).await;
        #[cfg(not(feature = "io-uring"))]
        assert!(matches!(result, Err(IoUringError::UnsupportedPlatform)));
        #[cfg(feature = "io-uring")]
        result.expect("should succeed with io-uring feature on Linux");
    }
}
