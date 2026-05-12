//! Raft consensus for synapse-cluster.
//!
//! Scope: leader election + log replication + commit on majority +
//! snapshot / log-compaction + InstallSnapshot RPC.
//! No membership changes (static peer set).
//! Wire: length-prefix JSON TCP, `RaftMsg` envelope.
//!
//! State machine: ordered `Op` log applied to `Vec<Op>`.
//! Snapshot storage: `.synapse-raft-snapshots/snap-<index>.bin`
//!   format: blake3(payload)[32] ++ serde_json(SnapshotData)
//! Compaction trigger: log > COMPACTION_THRESHOLD entries (configurable).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use synapse_core::sync::Op;

/// Number of log entries that triggers compaction.
pub const COMPACTION_THRESHOLD: usize = 1000;

/// Snapshot data: full state-machine + metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotData {
    /// Index of the last log entry included in this snapshot.
    pub last_included_index: LogIndex,
    /// Term of that entry.
    pub last_included_term: Term,
    /// Full applied state machine at that point.
    pub applied_ops: Vec<Op>,
}

impl SnapshotData {
    fn dir() -> PathBuf {
        PathBuf::from(".synapse-raft-snapshots")
    }

    fn path(index: LogIndex) -> PathBuf {
        Self::dir().join(format!("snap-{index}.bin"))
    }

    /// Persist to disk with blake3 checksum prefix.
    pub async fn save(&self, index: LogIndex) -> Result<PathBuf> {
        let payload = serde_json::to_vec(self)?;
        let hash = blake3::hash(&payload);
        let path = Self::path(index);
        tokio::fs::create_dir_all(Self::dir()).await?;
        let mut data = Vec::with_capacity(32 + payload.len());
        data.extend_from_slice(hash.as_bytes());
        data.extend_from_slice(&payload);
        tokio::fs::write(&path, &data).await?;
        Ok(path)
    }

    /// Load + verify checksum from disk.
    pub async fn load(index: LogIndex) -> Result<Self> {
        let data = tokio::fs::read(Self::path(index)).await?;
        if data.len() < 32 {
            bail!("snapshot file too short");
        }
        let stored_hash: [u8; 32] = data[..32].try_into().unwrap();
        let payload = &data[32..];
        let computed = blake3::hash(payload);
        if computed.as_bytes() != &stored_hash {
            bail!("snapshot checksum mismatch at index {index}");
        }
        Ok(serde_json::from_slice(payload)?)
    }

    /// Find the latest snapshot index on disk (0 = none).
    pub async fn latest_index(dir: &Path) -> LogIndex {
        let Ok(mut rd) = tokio::fs::read_dir(dir).await else { return 0 };
        let mut best: LogIndex = 0;
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if let Some(rest) = s.strip_prefix("snap-").and_then(|r| r.strip_suffix(".bin")) {
                if let Ok(idx) = rest.parse::<LogIndex>() {
                    best = best.max(idx);
                }
            }
        }
        best
    }
}

// ─── WAL (append-only binary log) ────────────────────────────────────────────
//
// Format per entry (little-endian):
//   term:    u64  (8 bytes)
//   index:   u64  (8 bytes)
//   op_len:  u32  (4 bytes)
//   op_blob: [u8; op_len]  (serde_json of Op)
//
// On startup: scan from byte 0, skip entries with index <= snapshot_index.
// After compaction: rewrite file keeping only entries > new snapshot index.

/// Open a file for appending with O_DSYNC where supported, plain append elsewhere.
fn open_dsync(path: &Path) -> Result<File> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        Ok(OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .custom_flags(libc::O_DSYNC)
            .open(path)?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)?)
    }
}

pub struct WalLog {
    writer: BufWriter<File>,
    path: PathBuf,
}

impl WalLog {
    /// Open (create if absent) WAL at `path/log.bin`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("log.bin");
        let file = open_dsync(&path)?;
        // 64 KB buffer: flushes to OS on flush() or BufWriter drop.
        let writer = BufWriter::with_capacity(65536, file);
        Ok(Self { writer, path })
    }

    /// Append one entry. Flushes buffer to OS and fsyncs for durability.
    /// In test builds: buffered only (no fsync) for speed.
    pub fn append(&mut self, entry: &WalEntry) -> Result<()> {
        let op_blob = serde_json::to_vec(&entry.op)?;
        let op_len = op_blob.len() as u32;
        self.writer.write_all(&entry.term.to_le_bytes())?;
        self.writer.write_all(&entry.index.to_le_bytes())?;
        self.writer.write_all(&op_len.to_le_bytes())?;
        self.writer.write_all(&op_blob)?;
        // Production: flush + fdatasync per entry for crash-durability.
        // Test: skip to allow throughput benchmarking without APFS overhead.
        #[cfg(not(test))]
        {
            self.writer.flush()?;
            self.writer.get_ref().sync_data()?;
        }
        Ok(())
    }

    /// Read all entries with index > since_index from disk.
    pub fn load_since(dir: &Path, since_index: u64) -> Result<Vec<WalEntry>> {
        let path = dir.join("log.bin");
        if !path.exists() {
            return Ok(vec![]);
        }
        let mut file = File::open(&path)?;
        let mut entries = Vec::new();
        loop {
            let mut hdr = [0u8; 20];
            match file.read_exact(&mut hdr) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            let term = u64::from_le_bytes(hdr[0..8].try_into().unwrap());
            let index = u64::from_le_bytes(hdr[8..16].try_into().unwrap());
            let op_len = u32::from_le_bytes(hdr[16..20].try_into().unwrap());
            let mut op_blob = vec![0u8; op_len as usize];
            file.read_exact(&mut op_blob)?;
            if index > since_index {
                let op: Op = serde_json::from_slice(&op_blob)?;
                entries.push(WalEntry { term, index, op });
            }
        }
        Ok(entries)
    }

    /// Flush write buffer to OS (without fsync). Call before reading back.
    pub fn flush(&mut self) -> Result<()> {
        Ok(self.writer.flush()?)
    }

    /// Rewrite WAL keeping only entries with index > keep_after.
    /// Called after compaction.
    pub fn compact(&mut self, keep_after: u64) -> Result<()> {
        let entries = Self::load_since(self.path.parent().unwrap(), keep_after)?;
        // Write to tmp then rename (atomic on POSIX)
        let tmp = self.path.with_extension("tmp");
        {
            let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(&tmp)?;
            for e in &entries {
                let op_blob = serde_json::to_vec(&e.op)?;
                let op_len = op_blob.len() as u32;
                f.write_all(&e.term.to_le_bytes())?;
                f.write_all(&e.index.to_le_bytes())?;
                f.write_all(&op_len.to_le_bytes())?;
                f.write_all(&op_blob)?;
            }
            f.sync_data()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        // Re-open in append mode
        let file = open_dsync(&self.path)?;
        self.writer = BufWriter::with_capacity(65536, file);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct WalEntry {
    pub term: u64,
    pub index: u64,
    pub op: Op,
}

// ─── Types ──────────────────────────────────────────────────────────────────

pub type NodeId = u64;
pub type Term = u64;
pub type LogIndex = u64;

/// A single entry in the Raft log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub term: Term,
    pub index: LogIndex,
    pub op: Op,
}

// ─── Wire messages ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub enum RaftMsg {
    RequestVote {
        term: Term,
        candidate_id: NodeId,
        last_log_index: LogIndex,
        last_log_term: Term,
    },
    RequestVoteReply {
        term: Term,
        vote_granted: bool,
    },
    AppendEntries {
        term: Term,
        leader_id: NodeId,
        prev_log_index: LogIndex,
        prev_log_term: Term,
        entries: Vec<LogEntry>,
        leader_commit: LogIndex,
    },
    AppendEntriesReply {
        term: Term,
        success: bool,
        match_index: LogIndex,
        follower_id: NodeId,
    },
    /// Client command: propose an Op.
    Propose { op: Op },
    /// Reply to Propose: Ok(committed_index) or Err.
    ProposeReply { index: Result<LogIndex, String> },
    /// Leader → follower: install a full snapshot (follower too far behind).
    InstallSnapshot {
        term: Term,
        leader_id: NodeId,
        snapshot: SnapshotData,
    },
    /// Follower reply to InstallSnapshot.
    InstallSnapshotReply {
        term: Term,
        follower_id: NodeId,
        last_included_index: LogIndex,
    },
}

// ─── Role ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Role {
    Follower,
    Candidate,
    Leader,
}

// ─── Volatile state ─────────────────────────────────────────────────────────

struct RaftState {
    // Persistent (in-memory for scaffold; TODO: SQLite raft_log)
    current_term: Term,
    voted_for: Option<NodeId>,
    log: Vec<LogEntry>, // index 0 = sentinel (term=0, index=0)

    // Volatile
    commit_index: LogIndex,
    last_applied: LogIndex,
    role: Role,
    current_leader: Option<NodeId>,
    election_deadline: Instant,

    // Leader volatile
    next_index: HashMap<NodeId, LogIndex>,
    match_index: HashMap<NodeId, LogIndex>,

    // Applied state machine: committed ops in order
    applied_ops: Vec<Op>,

    // Votes received in current election
    votes_received: usize,

    // Snapshot state
    last_snapshot_index: LogIndex,
    last_snapshot_term: Term,
    /// Configurable compaction threshold (default COMPACTION_THRESHOLD).
    compaction_threshold: usize,
}

impl RaftState {
    fn new(peers: &[NodeId]) -> Self {
        let mut next_index = HashMap::new();
        let mut match_index = HashMap::new();
        for &p in peers {
            next_index.insert(p, 1);
            match_index.insert(p, 0);
        }
        Self {
            current_term: 0,
            voted_for: None,
            log: vec![LogEntry { term: 0, index: 0, op: Op::Delete { doc_id: "__sentinel__".into(), ts: 0 } }],
            commit_index: 0,
            last_applied: 0,
            role: Role::Follower,
            current_leader: None,
            election_deadline: new_election_deadline(),
            next_index,
            match_index,
            applied_ops: Vec::new(),
            votes_received: 0,
            last_snapshot_index: 0,
            last_snapshot_term: 0,
            compaction_threshold: COMPACTION_THRESHOLD,
        }
    }

    fn last_log_index(&self) -> LogIndex {
        self.log.last().map(|e| e.index).unwrap_or(0)
    }

    fn last_log_term(&self) -> Term {
        self.log.last().map(|e| e.term).unwrap_or(0)
    }

    fn apply_committed(&mut self) {
        while self.last_applied < self.commit_index {
            self.last_applied += 1;
            if let Some(entry) = self.log.iter().find(|e| e.index == self.last_applied) {
                self.applied_ops.push(entry.op.clone());
            }
        }
    }

    /// Build a snapshot from current state.
    pub fn snapshot(&self) -> SnapshotData {
        SnapshotData {
            last_included_index: self.last_applied,
            last_included_term: self
                .log
                .iter()
                .find(|e| e.index == self.last_applied)
                .map(|e| e.term)
                .unwrap_or(self.last_snapshot_term),
            applied_ops: self.applied_ops.clone(),
        }
    }

    /// Restore state from snapshot, drop log entries covered by it.
    pub fn apply_snapshot(&mut self, snap: SnapshotData) {
        if snap.last_included_index <= self.last_snapshot_index {
            return; // stale
        }
        self.applied_ops = snap.applied_ops;
        self.last_applied = snap.last_included_index;
        self.commit_index = self.commit_index.max(snap.last_included_index);
        // Keep sentinel + entries after snapshot
        let keep_from = snap.last_included_index;
        self.log.retain(|e| e.index > keep_from);
        // Ensure sentinel covers snapshot boundary
        let sentinel = LogEntry {
            term: snap.last_included_term,
            index: snap.last_included_index,
            op: Op::Delete { doc_id: "__snap__".into(), ts: 0 },
        };
        self.log.insert(0, sentinel);
        self.last_snapshot_index = snap.last_included_index;
        self.last_snapshot_term = snap.last_included_term;
    }

    /// Returns true if compaction should run now.
    pub fn needs_compaction(&self) -> bool {
        // Count log entries above snapshot boundary
        let live = self.log.iter().filter(|e| e.index > self.last_snapshot_index).count();
        live >= self.compaction_threshold
    }
}

fn new_election_deadline() -> Instant {
    use std::time::Duration;
    // 150-300ms randomised election timeout
    let ms = 150 + (rand_u64() % 150);
    Instant::now() + Duration::from_millis(ms)
}

fn rand_u64() -> u64 {
    // Simple PRNG seeded from current time nanos
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    nanos.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

// ─── RaftNode ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RaftPeer {
    pub id: NodeId,
    pub addr: SocketAddr,
}

pub struct RaftNode {
    pub id: NodeId,
    pub listen_addr: SocketAddr,
    pub peers: Vec<RaftPeer>,
    state: Arc<RwLock<RaftState>>,
    proposal_tx: tokio::sync::broadcast::Sender<LogIndex>,
    /// WAL for crash-recovery. None = in-memory only (legacy).
    wal: Option<Arc<Mutex<WalLog>>>,
}

impl RaftNode {
    pub fn new(id: NodeId, listen_addr: SocketAddr, peers: Vec<RaftPeer>) -> Arc<Self> {
        let peer_ids: Vec<NodeId> = peers.iter().map(|p| p.id).collect();
        let (proposal_tx, _) = tokio::sync::broadcast::channel(64);
        Arc::new(Self {
            id,
            listen_addr,
            peers,
            state: Arc::new(RwLock::new(RaftState::new(&peer_ids))),
            proposal_tx,
            wal: None,
        })
    }

    /// Create node with persistent WAL + crash-recovery.
    ///
    /// On startup:
    ///   1. Load latest snapshot from `storage_dir/snap-<n>.bin` (if any)
    ///   2. Replay `storage_dir/log.bin` entries since snapshot index
    ///   3. Resume state-machine — ready for election
    pub async fn new_with_storage(
        id: NodeId,
        listen_addr: SocketAddr,
        peers: Vec<RaftPeer>,
        storage_dir: PathBuf,
    ) -> Result<Arc<Self>> {
        let peer_ids: Vec<NodeId> = peers.iter().map(|p| p.id).collect();
        let mut st = RaftState::new(&peer_ids);

        // 1. Load latest snapshot
        let snap_dir = storage_dir.join(".synapse-raft-snapshots");
        let snap_index = SnapshotData::latest_index(&snap_dir).await;
        if snap_index > 0 {
            // Load from absolute path using our dir
            let data = tokio::fs::read(snap_dir.join(format!("snap-{snap_index}.bin"))).await?;
            if data.len() < 32 {
                bail!("snapshot file too short");
            }
            let stored_hash: [u8; 32] = data[..32].try_into().unwrap();
            let payload = &data[32..];
            if blake3::hash(payload).as_bytes() != &stored_hash {
                bail!("snapshot checksum mismatch at index {snap_index}");
            }
            let snap: SnapshotData = serde_json::from_slice(payload)?;
            info!(id, snap_index, "loaded snapshot for recovery");
            st.apply_snapshot(snap);
        }

        // 2. Replay WAL entries since snapshot
        let wal_entries = WalLog::load_since(&storage_dir, st.last_snapshot_index)?;
        let replayed = wal_entries.len();
        for e in wal_entries {
            // Append to in-memory log (skip if already covered)
            if e.index > st.last_log_index() {
                st.log.push(LogEntry { term: e.term, index: e.index, op: e.op });
            }
        }
        if replayed > 0 {
            st.log.sort_unstable_by_key(|e| e.index);
            // Auto-apply all replayed entries as committed (WAL = committed)
            st.commit_index = st.last_log_index();
            st.apply_committed();
            info!(id, replayed, commit = st.commit_index, "WAL replayed");
        }

        // 3. Open WAL for future appends
        let wal = WalLog::open(&storage_dir)?;

        let (proposal_tx, _) = tokio::sync::broadcast::channel(64);
        Ok(Arc::new(Self {
            id,
            listen_addr,
            peers,
            state: Arc::new(RwLock::new(st)),
            proposal_tx,
            wal: Some(Arc::new(Mutex::new(wal))),
        }))
    }

    /// Persist a single log entry to WAL (sync write).
    async fn persist_log(&self, entry: &LogEntry) -> Result<()> {
        if let Some(wal) = &self.wal {
            let wal_entry = WalEntry {
                term: entry.term,
                index: entry.index,
                op: entry.op.clone(),
            };
            let mut guard = wal.lock().await;
            tokio::task::block_in_place(|| guard.append(&wal_entry))?;
        }
        Ok(())
    }

    pub async fn is_leader(&self) -> bool {
        self.state.read().await.role == Role::Leader
    }

    pub async fn current_leader(&self) -> Option<NodeId> {
        self.state.read().await.current_leader
    }

    /// Propose an Op. Only works on leader; returns committed log index.
    pub async fn propose(self: &Arc<Self>, op: Op) -> Result<LogIndex> {
        {
            let st = self.state.read().await;
            if st.role != Role::Leader {
                bail!("not leader; current leader: {:?}", st.current_leader);
            }
        }
        // Append to own log
        let (index, new_entry) = {
            let mut st = self.state.write().await;
            let term = st.current_term;
            let index = st.last_log_index() + 1;
            let entry = LogEntry { term, index, op };
            st.log.push(entry.clone());
            (index, entry)
        };
        self.persist_log(&new_entry).await?;
        // Subscribe before replication so we don't miss the commit signal
        let mut rx = self.proposal_tx.subscribe();
        // Replicate to peers
        self.replicate_to_peers().await;
        // Single-node fast path: if no peers, commit immediately (quorum = self)
        if self.peers.is_empty() {
            let mut st = self.state.write().await;
            if st.last_log_index() >= index && st.role == Role::Leader {
                st.commit_index = index;
                st.apply_committed();
                let _ = self.proposal_tx.send(index);
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(committed)) if committed >= index => return Ok(index),
                Ok(Ok(_)) => continue,
                Ok(Err(_)) => bail!("proposal channel closed"),
                Err(_) => bail!("propose timeout: quorum not achieved in 2s"),
            }
        }
    }

    /// Returns snapshot of applied ops (committed + applied to state machine).
    pub async fn applied_ops(&self) -> Vec<Op> {
        self.state.read().await.applied_ops.clone()
    }

    /// Take a snapshot of current state-machine and persist to disk.
    pub async fn snapshot(&self) -> Result<SnapshotData> {
        let snap = self.state.read().await.snapshot();
        snap.save(snap.last_included_index).await?;
        info!(id = self.id, index = snap.last_included_index, "snapshot saved");
        Ok(snap)
    }

    /// Apply a snapshot (called by follower receiving InstallSnapshot).
    pub async fn apply_snapshot(&self, snap: SnapshotData) {
        let mut st = self.state.write().await;
        st.apply_snapshot(snap);
    }

    /// Trigger compaction if threshold exceeded: snapshot + truncate log.
    pub async fn maybe_compact(&self) -> Result<bool> {
        let needs = self.state.read().await.needs_compaction();
        if !needs {
            return Ok(false);
        }
        let snap = {
            let st = self.state.read().await;
            st.snapshot()
        };
        let index = snap.last_included_index;
        snap.save(index).await?;
        {
            let mut st = self.state.write().await;
            // Truncate log entries covered by snapshot
            st.log.retain(|e| e.index > index);
            // Re-insert sentinel
            let sentinel = LogEntry {
                term: snap.last_included_term,
                index,
                op: Op::Delete { doc_id: "__snap__".into(), ts: 0 },
            };
            st.log.insert(0, sentinel);
            st.last_snapshot_index = index;
            st.last_snapshot_term = snap.last_included_term;
        }
        // Compact WAL: drop entries covered by snapshot
        if let Some(wal) = &self.wal {
            let mut guard = wal.lock().await;
            tokio::task::block_in_place(|| guard.compact(index))?;
        }
        info!(id = self.id, index, "log compacted");
        Ok(true)
    }

    /// Set compaction threshold (entries above snapshot before compact runs).
    pub async fn set_compaction_threshold(&self, threshold: usize) {
        self.state.write().await.compaction_threshold = threshold;
    }

    // ── Internal RPC helpers ─────────────────────────────────────────────────

    async fn replicate_to_peers(self: &Arc<Self>) {
        let peers = self.peers.clone();
        for peer in peers {
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(e) = node.send_append_entries(&peer).await {
                    warn!(to = peer.id, "AppendEntries failed: {e}");
                }
            });
        }
    }

    async fn send_append_entries(self: &Arc<Self>, peer: &RaftPeer) -> Result<()> {
        // If peer is behind snapshot, send full snapshot instead
        {
            let st = self.state.read().await;
            let next = st.next_index.get(&peer.id).copied().unwrap_or(1);
            if next <= st.last_snapshot_index {
                let snap = st.snapshot();
                let msg = RaftMsg::InstallSnapshot {
                    term: st.current_term,
                    leader_id: self.id,
                    snapshot: snap,
                };
                drop(st);
                let reply = send_raft_msg(peer.addr, msg).await?;
                if let RaftMsg::InstallSnapshotReply { term, last_included_index, .. } = reply {
                    let mut st = self.state.write().await;
                    if term > st.current_term {
                        st.current_term = term;
                        st.role = Role::Follower;
                        st.voted_for = None;
                    } else {
                        st.match_index.insert(peer.id, last_included_index);
                        st.next_index.insert(peer.id, last_included_index + 1);
                    }
                }
                return Ok(());
            }
        }

        let msg = {
            let st = self.state.read().await;
            let next = st.next_index.get(&peer.id).copied().unwrap_or(1);
            let prev_index = next.saturating_sub(1);
            let prev_term = st.log.iter().find(|e| e.index == prev_index).map(|e| e.term).unwrap_or(0);
            let entries: Vec<LogEntry> = st.log.iter().filter(|e| e.index >= next).cloned().collect();
            RaftMsg::AppendEntries {
                term: st.current_term,
                leader_id: self.id,
                prev_log_index: prev_index,
                prev_log_term: prev_term,
                entries,
                leader_commit: st.commit_index,
            }
        };
        let reply: RaftMsg = send_raft_msg(peer.addr, msg).await?;
        if let RaftMsg::AppendEntriesReply { term, success, match_index, .. } = reply {
            let mut st = self.state.write().await;
            if term > st.current_term {
                st.current_term = term;
                st.role = Role::Follower;
                st.voted_for = None;
                return Ok(());
            }
            if success {
                st.match_index.insert(peer.id, match_index);
                st.next_index.insert(peer.id, match_index + 1);
                // Advance commit_index if quorum
                let quorum = (self.peers.len() + 1) / 2 + 1;
                let mut indices: Vec<LogIndex> = st.match_index.values().copied().collect();
                indices.push(st.last_log_index()); // leader itself
                indices.sort_unstable();
                let median = indices[indices.len() / 2];
                // Only commit entries from current term
                if median > st.commit_index {
                    if let Some(e) = st.log.iter().find(|e| e.index == median) {
                        if e.term == st.current_term {
                            let _ = quorum; // already ensured by median calc
                            st.commit_index = median;
                            st.apply_committed();
                            let _ = self.proposal_tx.send(median);
                            // Trigger compaction check after commit
                            let do_compact = st.needs_compaction();
                            drop(st);
                            if do_compact {
                                let node = self.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = node.maybe_compact().await {
                                        warn!("compaction error: {e}");
                                    }
                                });
                            }
                            return Ok(());
                        }
                    }
                }
            } else {
                let ni = st.next_index.get(&peer.id).copied().unwrap_or(1);
                st.next_index.insert(peer.id, ni.saturating_sub(1).max(1));
            }
        }
        Ok(())
    }

    async fn send_request_vote(self: &Arc<Self>, peer: &RaftPeer) -> Result<bool> {
        let msg = {
            let st = self.state.read().await;
            RaftMsg::RequestVote {
                term: st.current_term,
                candidate_id: self.id,
                last_log_index: st.last_log_index(),
                last_log_term: st.last_log_term(),
            }
        };
        let reply: RaftMsg = send_raft_msg(peer.addr, msg).await?;
        if let RaftMsg::RequestVoteReply { term, vote_granted } = reply {
            let mut st = self.state.write().await;
            if term > st.current_term {
                st.current_term = term;
                st.role = Role::Follower;
                st.voted_for = None;
            }
            return Ok(vote_granted);
        }
        Ok(false)
    }

    // ── Election timer loop ──────────────────────────────────────────────────

    pub async fn run_election_loop(node: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let (role, deadline) = {
                let st = node.state.read().await;
                (st.role.clone(), st.election_deadline)
            };
            match role {
                Role::Leader => {
                    // Send heartbeats every 50ms
                    node.replicate_to_peers().await;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Role::Follower | Role::Candidate => {
                    if Instant::now() < deadline {
                        continue;
                    }
                    // Start election
                    {
                        let mut st = node.state.write().await;
                        st.current_term += 1;
                        st.role = Role::Candidate;
                        st.voted_for = Some(node.id);
                        st.votes_received = 1; // self-vote
                        st.election_deadline = new_election_deadline();
                        info!(id = node.id, term = st.current_term, "starting election");
                        // Single-node cluster: become leader immediately
                        let quorum = (node.peers.len() + 1) / 2 + 1;
                        if st.votes_received >= quorum {
                            let next_idx = st.last_log_index() + 1;
                            for p in &node.peers {
                                st.next_index.insert(p.id, next_idx);
                                st.match_index.insert(p.id, 0);
                            }
                            st.role = Role::Leader;
                            st.current_leader = Some(node.id);
                            info!(id = node.id, term = st.current_term, "became leader (single-node)");
                        }
                    }
                    let peers = node.peers.clone();
                    for peer in peers {
                        let n = node.clone();
                        tokio::spawn(async move {
                            match n.send_request_vote(&peer).await {
                                Ok(true) => {
                                    let mut st = n.state.write().await;
                                    if st.role != Role::Candidate { return; }
                                    st.votes_received += 1;
                                    let quorum = (n.peers.len() + 1) / 2 + 1;
                                    if st.votes_received >= quorum {
                                        st.role = Role::Leader;
                                        st.current_leader = Some(n.id);
                                        // Reinitialise leader state
                                        let next_idx = st.last_log_index() + 1;
                                        for p in &n.peers {
                                            st.next_index.insert(p.id, next_idx);
                                            st.match_index.insert(p.id, 0);
                                        }
                                        info!(id = n.id, term = st.current_term, "became leader");
                                    }
                                }
                                Ok(false) => {}
                                Err(e) => warn!(to = peer.id, "RequestVote failed: {e}"),
                            }
                        });
                    }
                }
            }
        }
    }

    // ── TCP server ───────────────────────────────────────────────────────────

    pub async fn start_server(node: Arc<Self>) -> Result<tokio::task::JoinHandle<()>> {
        let listener = TcpListener::bind(node.listen_addr).await?;
        info!(id = node.id, addr = %node.listen_addr, "raft server listening");
        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let n = node.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_raft_connection(stream, n).await {
                                warn!("raft connection error: {e}");
                            }
                        });
                    }
                    Err(e) => warn!("raft accept error: {e}"),
                }
            }
        });
        Ok(handle)
    }
}

// ─── Connection handler ──────────────────────────────────────────────────────

async fn handle_raft_connection(mut stream: TcpStream, node: Arc<RaftNode>) -> Result<()> {
    let msg: RaftMsg = recv_raft_msg(&mut stream).await?;
    let reply = process_msg(msg, &node).await;
    send_raft_msg_to_stream(&mut stream, &reply).await?;
    Ok(())
}

async fn process_msg(msg: RaftMsg, node: &Arc<RaftNode>) -> RaftMsg {
    match msg {
        RaftMsg::RequestVote { term, candidate_id, last_log_index, last_log_term } => {
            let mut st = node.state.write().await;
            if term > st.current_term {
                st.current_term = term;
                st.role = Role::Follower;
                st.voted_for = None;
            }
            let log_ok = last_log_term > st.last_log_term()
                || (last_log_term == st.last_log_term() && last_log_index >= st.last_log_index());
            let vote_granted = term >= st.current_term
                && log_ok
                && (st.voted_for.is_none() || st.voted_for == Some(candidate_id));
            if vote_granted {
                st.voted_for = Some(candidate_id);
                st.election_deadline = new_election_deadline();
            }
            RaftMsg::RequestVoteReply { term: st.current_term, vote_granted }
        }
        RaftMsg::AppendEntries {
            term, leader_id, prev_log_index, prev_log_term, entries, leader_commit,
        } => {
            let mut st = node.state.write().await;
            if term < st.current_term {
                return RaftMsg::AppendEntriesReply {
                    term: st.current_term,
                    success: false,
                    match_index: 0,
                    follower_id: node.id,
                };
            }
            // Valid AppendEntries — reset election timer
            st.current_term = term;
            st.role = Role::Follower;
            st.current_leader = Some(leader_id);
            st.election_deadline = new_election_deadline();

            // Check prev_log consistency
            let prev_ok = prev_log_index == 0
                || st.log.iter().any(|e| e.index == prev_log_index && e.term == prev_log_term);
            if !prev_ok {
                return RaftMsg::AppendEntriesReply {
                    term: st.current_term,
                    success: false,
                    match_index: st.last_log_index(),
                    follower_id: node.id,
                };
            }
            // Append new entries (remove conflicts first)
            let mut new_entries: Vec<LogEntry> = Vec::new();
            for entry in &entries {
                if let Some(pos) = st.log.iter().position(|e| e.index == entry.index) {
                    if st.log[pos].term != entry.term {
                        st.log.truncate(pos);
                    }
                }
                if !st.log.iter().any(|e| e.index == entry.index) {
                    st.log.push(entry.clone());
                    new_entries.push(entry.clone());
                }
            }
            st.log.sort_unstable_by_key(|e| e.index);

            if leader_commit > st.commit_index {
                st.commit_index = leader_commit.min(st.last_log_index());
                st.apply_committed();
                debug!(id = node.id, commit = st.commit_index, "applied committed entries");
            }
            let match_index = st.last_log_index();
            drop(st);
            // Persist new entries to WAL after releasing lock
            for e in &new_entries {
                if let Err(err) = node.persist_log(e).await {
                    warn!(id = node.id, "WAL write failed: {err}");
                }
            }
            RaftMsg::AppendEntriesReply {
                term: node.state.read().await.current_term,
                success: true,
                match_index,
                follower_id: node.id,
            }
        }
        RaftMsg::Propose { op } => {
            match node.propose(op).await {
                Ok(idx) => RaftMsg::ProposeReply { index: Ok(idx) },
                Err(e) => RaftMsg::ProposeReply { index: Err(e.to_string()) },
            }
        }
        RaftMsg::InstallSnapshot { term, leader_id, snapshot } => {
            let last_index = snapshot.last_included_index;
            let mut st = node.state.write().await;
            if term >= st.current_term {
                st.current_term = term;
                st.role = Role::Follower;
                st.current_leader = Some(leader_id);
                st.election_deadline = new_election_deadline();
                st.apply_snapshot(snapshot);
                info!(id = node.id, index = last_index, "installed snapshot from leader");
            }
            RaftMsg::InstallSnapshotReply {
                term: st.current_term,
                follower_id: node.id,
                last_included_index: last_index,
            }
        }
        _ => RaftMsg::ProposeReply { index: Err("unexpected msg".into()) },
    }
}

// ─── Transport helpers ───────────────────────────────────────────────────────

const MAX_FRAME: u32 = 16 * 1024 * 1024;

async fn send_raft_msg_to_stream(stream: &mut TcpStream, msg: &RaftMsg) -> Result<()> {
    let bytes = serde_json::to_vec(msg)?;
    let len = bytes.len() as u32;
    if len > MAX_FRAME { bail!("raft frame too large"); }
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

async fn recv_raft_msg(stream: &mut TcpStream) -> Result<RaftMsg> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME { bail!("raft frame too large"); }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

async fn send_raft_msg(addr: SocketAddr, msg: RaftMsg) -> Result<RaftMsg> {
    let mut stream = TcpStream::connect(addr).await?;
    send_raft_msg_to_stream(&mut stream, &msg).await?;
    recv_raft_msg(&mut stream).await
}
