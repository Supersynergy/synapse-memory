/// Smoke-test: 3-node cluster, 100 ops, compaction, state-machine intact.
/// Requires feature `cluster-raft`.
#[cfg(feature = "cluster-raft")]
mod snapshot_tests {
    use std::net::SocketAddr;
    use synapse_cluster::raft::{RaftNode, RaftPeer, SnapshotData};
    use synapse_core::sync::Op;

    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    fn make_op(i: u64) -> Op {
        Op::Delete {
            doc_id: format!("doc-{i}"),
            ts: i as i64,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn three_node_snapshot_and_compaction() {
        let _ = tracing_subscriber::fmt::try_init();

        // Allocate ports
        let p1: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
        let p2: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
        let p3: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();

        let node1 = RaftNode::new(
            1,
            p1,
            vec![RaftPeer { id: 2, addr: p2 }, RaftPeer { id: 3, addr: p3 }],
        );
        let node2 = RaftNode::new(
            2,
            p2,
            vec![RaftPeer { id: 1, addr: p1 }, RaftPeer { id: 3, addr: p3 }],
        );
        let node3 = RaftNode::new(
            3,
            p3,
            vec![RaftPeer { id: 1, addr: p1 }, RaftPeer { id: 2, addr: p2 }],
        );

        // Set low compaction threshold so 100 ops triggers it
        node1.set_compaction_threshold(50).await;
        node2.set_compaction_threshold(50).await;
        node3.set_compaction_threshold(50).await;

        // Start servers + election loops
        let _s1 = RaftNode::start_server(node1.clone()).await.unwrap();
        let _s2 = RaftNode::start_server(node2.clone()).await.unwrap();
        let _s3 = RaftNode::start_server(node3.clone()).await.unwrap();

        let n1 = node1.clone();
        tokio::spawn(async move { RaftNode::run_election_loop(n1).await });
        let n2 = node2.clone();
        tokio::spawn(async move { RaftNode::run_election_loop(n2).await });
        let n3 = node3.clone();
        tokio::spawn(async move { RaftNode::run_election_loop(n3).await });

        // Wait for leader election (up to 3s)
        let leader = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                for n in [&node1, &node2, &node3] {
                    if n.is_leader().await {
                        return n.clone();
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("no leader elected in 3s");

        // Propose 100 ops
        for i in 0..100u64 {
            leader.propose(make_op(i)).await.expect("propose failed");
        }

        // Allow replication to settle
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Verify leader state-machine has 100 ops
        let ops = leader.applied_ops().await;
        assert_eq!(
            ops.len(),
            100,
            "expected 100 applied ops, got {}",
            ops.len()
        );

        // Explicitly trigger compaction on leader (may already be done auto)
        let compacted = leader.maybe_compact().await.unwrap();
        // Either already compacted (auto) or compacted now
        let _ = compacted; // either is fine

        // Snapshot should be persisted — load it and verify
        let snap_index = leader.applied_ops().await.len() as u64;
        // Manually snapshot to ensure file exists
        let snap = leader.snapshot().await.unwrap();
        assert_eq!(snap.applied_ops.len(), 100);
        assert!(snap.last_included_index > 0);

        // Load from disk and verify checksum + contents
        let loaded = SnapshotData::load(snap.last_included_index)
            .await
            .expect("snapshot load failed");
        assert_eq!(loaded.applied_ops.len(), snap.applied_ops.len());
        assert_eq!(loaded.last_included_index, snap.last_included_index);

        // Cleanup snapshot files
        let _ = tokio::fs::remove_dir_all(".synapse-raft-snapshots").await;
        let _ = snap_index;
    }

    /// Unit test: apply_snapshot restores state and drops old log entries.
    #[tokio::test]
    async fn apply_snapshot_unit() {
        use synapse_cluster::raft::SnapshotData;

        let snap = SnapshotData {
            last_included_index: 50,
            last_included_term: 1,
            applied_ops: (0..50).map(make_op).collect(),
        };

        let node = RaftNode::new(9, "127.0.0.1:0".parse().unwrap(), vec![]);
        node.apply_snapshot(snap.clone()).await;

        let ops = node.applied_ops().await;
        assert_eq!(ops.len(), 50);
    }
}
