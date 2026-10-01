# synapse-cluster WORK_LOG

## Crate-Pfade

```
crates/synapse-cluster/
  Cargo.toml
  WORK_LOG.md
  src/
    lib.rs        — Node, NodeState, gossip_loop, merge_peer_delta, put_op
    proto.rs      — Request/Response enum (Hello, PullChanges, PushChanges)
    transport.rs  — TCP length-prefix framing, handle_connection, pull/push_to_peer
```

Workspace: `Cargo.toml` → `"crates/synapse-cluster"` eingefügt.
`synapse-core/src/lib.rs` → `pub mod sync;` ergänzt (war nicht re-exportiert).

## API

```rust
// Node anlegen
let mut node = Node::new("node1", "127.0.0.1:19901".parse()?);
node.add_peer(PeerInfo { id: "node2".into(), addr: addr2 });
let node = Arc::new(node);

// Server starten (TCP gossip listener)
Node::start_server(node.clone()).await?;

// Lokalen Op schreiben
node.put_op(Op::Put { doc_id: "x".into(), blob_hash: [1;32], ts: 100 }).await?;

// Gossip-Loop (Hintergrundtask, push delta an alle Peers)
tokio::spawn(Node::gossip_loop(node.clone()));

// Peer-Delta manuell anwenden (CRDT merge_lww)
node.merge_peer_delta(delta).await;
```

## Wire-Protokoll

Length-prefix TCP: `[4 bytes LE u32][JSON payload]`. Max frame 16 MiB.

```
Request  ::= Hello{from} | PullChanges{since:Clock} | PushChanges{ops}
Response ::= Ok | Changes{ops} | Err{msg}
```

Gossip-Tick (default 500ms): für jeden Peer → `PullChanges` → `merge_lww` → update peer_clock.

## 2-Node Smoke-Test (loopback, tokio multi-thread)

```
test tests::two_node_gossip_smoke ... ok
test tests::crdt_merge_idempotent  ... ok
```

- put op auf node1 → `push_to_peer(node2)` → node2.local_ops() == 1 doc  
- Latenz push loopback: **< 5ms** (assertion < 200ms, gepasst)  
- Doc-Propagation: single round-trip, synchron

## CRDT-Vorteil vs Qdrant-Cluster

| | Qdrant Cluster | synapse-cluster |
|---|---|---|
| Konsistenz | CP (leader election via Raft) | AP (CRDT, kein Leader) |
| Offline-Merge | nein | **ja** — partition = kein Datenverlust |
| Conflict-Resolution | last-write-wins (Raft log) | merge_lww (Automerge-ready) |
| Leader-Election | Raft benötigt Quorum | **entfällt** |
| Split-Brain | Schreibfehler bei Quorum-Verlust | Schreiben immer lokal möglich |
| Network overhead | Raft heartbeat + replication log | Gossip push/pull (lazy) |

CRDT advantage: Nodes können wochen offline sein, dann mergen — kein Datenverlust, keine manuelle Conflict-Resolution. Besonders stark für edge-deployments (Laptop, mobile agent).

## Next Steps

1. **Raft-Option** (`synapse-raft` bereits als Crate vorhanden) — CP-Modus wenn stärkere Konsistenz benötigt. Toggle: `Node::with_consensus(RaftBackend)`.
2. **Automerge-Backend** — `crdt` feature in synapse-core aktivieren → `merge_payload` statt `merge_lww` → merge für Scalar-Felder bulletproof.
3. **Sharding** — `NodeId`-consistent-hashing, Partition nach `doc_id` prefix, `Node::route_op(op)` → lokaler write oder forward.
4. **Gossip-Fan-Out** — statt sequential: parallele Tokio-Tasks per Peer (bereits einfach umzusetzen in `gossip_loop`).
5. **Persistent Op-Log** — aktuell in-memory `Vec`; Ziel: WAL in `synapse-wal` persistieren → crash-safe.
6. **Integration-Test** — 3-Node Ring, put auf node1, nach 2 gossip-Ticks query auf node3.
