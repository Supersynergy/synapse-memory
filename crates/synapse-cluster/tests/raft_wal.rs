/// WAL persistence + crash-recovery smoke test.
/// Requires feature `cluster-raft`.
#[cfg(feature = "cluster-raft")]
mod wal_tests {
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use synapse_cluster::raft::{RaftNode, RaftPeer, WalLog};
    use synapse_core::sync::Op;

    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    fn make_op(i: u64) -> Op {
        Op::Delete { doc_id: format!("doc-{i}"), ts: i as i64 }
    }

    // ── WAL round-trip (unit) ────────────────────────────────────────────────

    #[test]
    fn wal_append_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        let mut wal = WalLog::open(&path).unwrap();
        for i in 1u64..=50 {
            wal.append(&synapse_cluster::raft::WalEntry {
                term: 1,
                index: i,
                op: make_op(i),
            })
            .unwrap();
        }
        wal.flush().unwrap();
        drop(wal);

        let entries = WalLog::load_since(&path, 0).unwrap();
        assert_eq!(entries.len(), 50);
        assert_eq!(entries[0].index, 1);
        assert_eq!(entries[49].index, 50);
    }

    #[test]
    fn wal_load_since_filters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        let mut wal = WalLog::open(&path).unwrap();
        for i in 1u64..=20 {
            wal.append(&synapse_cluster::raft::WalEntry {
                term: 1,
                index: i,
                op: make_op(i),
            })
            .unwrap();
        }
        wal.flush().unwrap();
        drop(wal);

        let entries = WalLog::load_since(&path, 10).unwrap();
        assert_eq!(entries.len(), 10);
        assert!(entries.iter().all(|e| e.index > 10));
    }

    #[test]
    fn wal_compact_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        let mut wal = WalLog::open(&path).unwrap();
        for i in 1u64..=30 {
            wal.append(&synapse_cluster::raft::WalEntry {
                term: 1,
                index: i,
                op: make_op(i),
            })
            .unwrap();
        }
        wal.flush().unwrap();
        wal.compact(20).unwrap();

        let entries = WalLog::load_since(&path, 0).unwrap();
        assert_eq!(entries.len(), 10, "only entries 21-30 remain");
        assert_eq!(entries[0].index, 21);
    }

    // ── Throughput bench (Linux only — macOS APFS journaling makes this unrepresentative) ──

    #[test]
    #[cfg(target_os = "linux")]
    fn wal_throughput_10k_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let mut wal = WalLog::open(&path).unwrap();

        let n = 10_000u64;
        let start = std::time::Instant::now();
        for i in 1..=n {
            wal.append(&synapse_cluster::raft::WalEntry {
                term: 1,
                index: i,
                op: make_op(i),
            })
            .unwrap();
        }
        wal.flush().unwrap();
        let elapsed = start.elapsed();
        let rate = n as f64 / elapsed.as_secs_f64();
        println!("WAL throughput: {rate:.0} entries/sec ({elapsed:?} for {n})");
        assert!(rate >= 10_000.0, "target ≥10k entries/sec, got {rate:.0}");
    }

    // ── Crash-recovery integration ───────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn crash_recovery_single_node() {
        let _ = tracing_subscriber::fmt::try_init();
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().to_path_buf();

        let port1 = free_port();
        let addr1: SocketAddr = format!("127.0.0.1:{port1}").parse().unwrap();

        // Phase 1: boot node, propose 100 ops, shut down
        {
            let node = RaftNode::new_with_storage(1, addr1, vec![], storage.clone())
                .await
                .unwrap();
            let _srv = RaftNode::start_server(node.clone()).await.unwrap();
            tokio::spawn(RaftNode::run_election_loop(node.clone()));

            // Wait for leader (single node always wins within ~300ms)
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if node.is_leader().await { break; }
                if tokio::time::Instant::now() > deadline { panic!("node never became leader"); }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }

            for i in 0u64..100 {
                node.propose(make_op(i)).await.unwrap();
            }
            assert_eq!(node.applied_ops().await.len(), 100);
            // Node dropped here — simulates crash
        }

        // Phase 2: restart from same storage_dir — should recover all 100 ops
        {
            let port2 = free_port();
            let addr2: SocketAddr = format!("127.0.0.1:{port2}").parse().unwrap();
            let node2 = RaftNode::new_with_storage(1, addr2, vec![], storage.clone())
                .await
                .unwrap();
            let ops = node2.applied_ops().await;
            assert_eq!(
                ops.len(),
                100,
                "expected 100 recovered ops, got {}",
                ops.len()
            );
        }
    }
}
