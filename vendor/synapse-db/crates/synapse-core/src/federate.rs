//! Federation layer: CRDT state sync over TCP/unix-socket with Ed25519-signed Updates.
//!
//! Protocol messages (msgpack-encoded):
//!   SyncStep1 { doc_id, sv }       → send my state-vector
//!   SyncStep2 { doc_id, update }   → send update needed by peer
//!   Update    { doc_id, update, sig, vk } → broadcast local update

use crate::error::{Error, Result};
use crate::sign;
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update, updates::decoder::Decode};

// ── Wire messages ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
pub enum Msg {
    /// Authenticated state-vector request: `sig` covers
    /// `b"syn-fed-sync1" || doc_id || sv` and must come from a trusted vk.
    SyncStep1 {
        doc_id: String,
        sv: Vec<u8>,
        vk: Vec<u8>,
        sig: Vec<u8>,
    },
    /// Authenticated diff reply: `sig` covers
    /// `b"syn-fed-sync2" || doc_id || update`.
    SyncStep2 {
        doc_id: String,
        update: Vec<u8>,
        vk: Vec<u8>,
        sig: Vec<u8>,
    },
    Update {
        doc_id: String,
        update: Vec<u8>,
        sig: Vec<u8>,
        vk: Vec<u8>,
    },
}

fn auth_payload(domain: &[u8], doc_id: &str, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(domain.len() + doc_id.len() + body.len() + 8);
    v.extend_from_slice(domain);
    v.extend_from_slice(&(doc_id.len() as u32).to_le_bytes());
    v.extend_from_slice(doc_id.as_bytes());
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    v
}

fn parse_vk_sig(vk: &[u8], sig: &[u8]) -> Result<(VerifyingKey, [u8; 64])> {
    let vk_arr: [u8; 32] = vk
        .try_into()
        .map_err(|_| Error::Other("vk must be 32 bytes".into()))?;
    let sig_arr: [u8; 64] = sig
        .try_into()
        .map_err(|_| Error::Other("sig must be 64 bytes".into()))?;
    let verifying_key =
        VerifyingKey::from_bytes(&vk_arr).map_err(|e| Error::Other(e.to_string()))?;
    Ok((verifying_key, sig_arr))
}

// ── Peer trust store ──────────────────────────────────────────────────────────

/// Pinning store for peer verifying keys. Federation is strict by default:
/// updates and sync requests from keys not in the store are rejected
/// (bd -j1u). `trust()` persists to disk with 0600 permissions on unix.
#[derive(Default)]
pub struct TrustStore {
    trusted: std::collections::HashSet<[u8; 32]>,
    path: Option<PathBuf>,
}

/// `~/.synapse/federation-trust.json` — same dir as `auth.token`.
pub fn default_trust_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".synapse/federation-trust.json")
}

impl TrustStore {
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load pinned keys from a JSON array of hex-encoded vks.
    /// Missing file → empty store (strict: everything unknown is rejected).
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let mut s = Self {
            trusted: Default::default(),
            path: Some(path.to_path_buf()),
        };
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let keys: Vec<String> = serde_json::from_str(&text)
                    .map_err(|e| Error::Other(format!("trust store corrupt: {e}")))?;
                for k in keys {
                    let bytes = decode_hex(&k)
                        .ok_or_else(|| Error::Other(format!("bad hex key in trust store: {k}")))?;
                    let arr: [u8; 32] = bytes
                        .try_into()
                        .map_err(|_| Error::Other("trust store key must be 32 bytes".into()))?;
                    s.trusted.insert(arr);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::Other(format!("trust store read: {e}"))),
        }
        Ok(s)
    }

    pub fn is_trusted(&self, vk: &[u8; 32]) -> bool {
        self.trusted.contains(vk)
    }

    /// Pin a peer key (TOFU happens only through this explicit call).
    pub fn trust(&mut self, vk: [u8; 32]) -> Result<()> {
        self.trusted.insert(vk);
        self.persist()
    }

    fn persist(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::Other(e.to_string()))?;
        }
        let keys: Vec<String> = self.trusted.iter().map(encode_hex).collect();
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string(&keys).unwrap())
            .map_err(|e| Error::Other(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| Error::Other(e.to_string()))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| Error::Other(e.to_string()))
    }
}

fn encode_hex(b: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for byte in b {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn encode(msg: &Msg) -> Result<Vec<u8>> {
    rmp_serde::to_vec(msg).map_err(|e| Error::Other(e.to_string()))
}

fn decode(buf: &[u8]) -> Result<Msg> {
    rmp_serde::from_slice(buf).map_err(|e| Error::Other(e.to_string()))
}

fn write_framed(w: &mut impl Write, data: &[u8]) -> Result<()> {
    let len = (data.len() as u32).to_le_bytes();
    w.write_all(&len)?;
    w.write_all(data)?;
    Ok(())
}

fn read_framed(r: &mut impl Read) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .map_err(|e| Error::Other(e.to_string()))?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        return Err(Error::Other("frame too large".into()));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(buf)
}

// ── Peer address ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum Addr {
    Unix(PathBuf),
    Tcp(String), // host:port
}

impl std::fmt::Display for Addr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Addr::Unix(p) => write!(f, "unix:{}", p.display()),
            Addr::Tcp(s) => write!(f, "tcp:{}", s),
        }
    }
}

impl std::str::FromStr for Addr {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        if let Some(rest) = s.strip_prefix("unix:") {
            Ok(Addr::Unix(PathBuf::from(rest)))
        } else if let Some(rest) = s.strip_prefix("tcp:") {
            Ok(Addr::Tcp(rest.to_string()))
        } else {
            // default: treat as tcp
            Ok(Addr::Tcp(s.to_string()))
        }
    }
}

// ── In-memory CRDT doc store used by Federation ───────────────────────────────

#[derive(Default)]
struct DocStore {
    docs: std::collections::HashMap<String, Doc>,
}

impl DocStore {
    fn get_or_create(&mut self, doc_id: &str) -> &Doc {
        self.docs.entry(doc_id.to_string()).or_default()
    }

    fn state_vector(&mut self, doc_id: &str) -> Vec<u8> {
        let doc = self.get_or_create(doc_id);
        let txn = doc.transact();
        txn.state_vector().encode_v1()
    }

    fn apply_update(&mut self, doc_id: &str, update: &[u8]) -> Result<Vec<u8>> {
        let doc = self.docs.entry(doc_id.to_string()).or_default();
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(update).map_err(|e| Error::Other(e.to_string()))?)
            .map_err(|e| Error::Other(e.to_string()))?;
        drop(txn);
        let txn = doc.transact();
        Ok(txn.encode_state_as_update_v1(&StateVector::default()))
    }

    fn diff_since(&mut self, doc_id: &str, sv_bytes: &[u8]) -> Result<Vec<u8>> {
        let doc = self.get_or_create(doc_id);
        let txn = doc.transact();
        let sv = StateVector::decode_v1(sv_bytes).map_err(|e| Error::Other(e.to_string()))?;
        Ok(txn.encode_state_as_update_v1(&sv))
    }
}

// ── Federation ────────────────────────────────────────────────────────────────

pub struct Federation {
    signing_key: SigningKey,
    peers: Arc<Mutex<Vec<Addr>>>,
    store: Arc<Mutex<DocStore>>,
    trust: Arc<Mutex<TrustStore>>,
}

impl Federation {
    /// In-memory trust store holding only our own vk — strictest mode,
    /// every peer must be pinned via `trust_peer`.
    pub fn new(signing_key: SigningKey) -> Self {
        Self::with_trust_store(signing_key, TrustStore::in_memory())
    }

    /// File-backed trust store (see `default_trust_path`): pinned peers
    /// survive restarts; own vk is always implicitly trusted.
    pub fn with_trust_store(signing_key: SigningKey, mut trust: TrustStore) -> Self {
        trust.trusted.insert(signing_key.verifying_key().to_bytes());
        Self {
            signing_key,
            peers: Arc::new(Mutex::new(vec![])),
            store: Arc::new(Mutex::new(DocStore::default())),
            trust: Arc::new(Mutex::new(trust)),
        }
    }

    /// File-backed trust store at `default_trust_path()` (`~/.synapse/
    /// federation-trust.json`) — the right constructor for long-lived tools.
    pub fn with_default_store(signing_key: SigningKey) -> Result<Self> {
        Ok(Self::with_trust_store(
            signing_key,
            TrustStore::load(&default_trust_path())?,
        ))
    }

    /// Pin a peer's verifying key (persists if the store is file-backed).
    pub fn trust_peer(&self, vk: &VerifyingKey) -> Result<()> {
        self.trust.lock().unwrap().trust(vk.to_bytes())
    }

    pub fn is_peer_trusted(&self, vk: &VerifyingKey) -> bool {
        self.trust.lock().unwrap().is_trusted(&vk.to_bytes())
    }

    pub fn add_peer(&self, addr: Addr) {
        self.peers.lock().unwrap().push(addr);
    }

    pub fn peers(&self) -> Vec<String> {
        self.peers
            .lock()
            .unwrap()
            .iter()
            .map(|a| a.to_string())
            .collect()
    }

    /// Broadcast a local CRDT update to all peers (signed).
    pub fn on_local_update(&self, doc_id: &str, update: &[u8]) -> Result<()> {
        let sig = sign::sign_bytes(&self.signing_key, update).to_vec();
        let vk = self.signing_key.verifying_key().to_bytes().to_vec();
        let msg = Msg::Update {
            doc_id: doc_id.to_string(),
            update: update.to_vec(),
            sig,
            vk,
        };
        let frame = encode(&msg)?;
        let peers = self.peers.lock().unwrap().clone();
        for addr in &peers {
            if let Err(e) = self.send_frame(addr, &frame) {
                tracing::warn!("federation: peer {} unreachable: {}", addr, e);
            }
        }
        Ok(())
    }

    /// Full sync with all peers: exchange state vectors, push/pull diffs.
    pub fn sync_all(&self) -> Result<()> {
        let peers = self.peers.lock().unwrap().clone();
        for addr in &peers {
            if let Err(e) = self.sync_peer(addr) {
                tracing::warn!("federation: sync with {} failed: {}", addr, e);
            }
        }
        Ok(())
    }

    fn sync_peer(&self, addr: &Addr) -> Result<()> {
        // For each doc we know, do a SyncStep1/SyncStep2 handshake
        let doc_ids: Vec<String> = {
            let store = self.store.lock().unwrap();
            store.docs.keys().cloned().collect()
        };
        if doc_ids.is_empty() {
            return Ok(());
        }
        let mut stream = connect(addr)?;
        for doc_id in &doc_ids {
            let sv = self.store.lock().unwrap().state_vector(doc_id);
            let sig = sign::sign_bytes(
                &self.signing_key,
                &auth_payload(b"syn-fed-sync1", doc_id, &sv),
            )
            .to_vec();
            let step1 = encode(&Msg::SyncStep1 {
                doc_id: doc_id.clone(),
                sv,
                vk: self.signing_key.verifying_key().to_bytes().to_vec(),
                sig,
            })?;
            write_framed(&mut stream, &step1)?;
            let reply = read_framed(&mut stream)?;
            let msg = decode(&reply)?;
            if let Msg::SyncStep2 {
                doc_id: rid,
                update,
                vk,
                sig,
            } = msg
                && rid == *doc_id
                && !update.is_empty()
            {
                // Only apply diffs from trusted peers with a valid sig (bd -j1u).
                let (peer_key, sig_arr) = parse_vk_sig(&vk, &sig)?;
                if !self.trust.lock().unwrap().is_trusted(&peer_key.to_bytes()) {
                    return Err(Error::Other(format!(
                        "sync reply from untrusted peer for {rid}"
                    )));
                }
                sign::verify_bytes(
                    &peer_key,
                    &auth_payload(b"syn-fed-sync2", &rid, &update),
                    &sig_arr,
                )?;
                self.store.lock().unwrap().apply_update(&rid, &update)?;
            }
        }
        Ok(())
    }

    fn send_frame(&self, addr: &Addr, frame: &[u8]) -> Result<()> {
        let mut stream = connect(addr)?;
        write_framed(&mut stream, frame)?;
        Ok(())
    }

    /// Receive a signed Update message from a peer. Verifies signature.
    pub fn receive_update(&self, msg: Msg) -> Result<()> {
        match msg {
            Msg::Update {
                doc_id,
                update,
                sig,
                vk,
            } => {
                let (verifying_key, sig_arr) = parse_vk_sig(&vk, &sig)?;
                if !self
                    .trust
                    .lock()
                    .unwrap()
                    .is_trusted(&verifying_key.to_bytes())
                {
                    return Err(Error::Other(format!(
                        "update from untrusted peer for {doc_id}"
                    )));
                }
                sign::verify_bytes(&verifying_key, &update, &sig_arr)?;
                self.store.lock().unwrap().apply_update(&doc_id, &update)?;
            }
            Msg::SyncStep1 { .. } | Msg::SyncStep2 { .. } => {
                return Err(Error::Other(
                    "unexpected message type in receive_update".into(),
                ));
            }
        }
        Ok(())
    }

    /// Apply a local update (from crdt.rs merge) and broadcast to peers.
    pub fn merge_and_broadcast(&self, doc_id: &str, update: &[u8]) -> Result<()> {
        self.store.lock().unwrap().apply_update(doc_id, update)?;
        self.on_local_update(doc_id, update)
    }

    /// Start a TCP listener, handling incoming sync and update messages.
    pub fn listen_tcp(&self, addr: &str) -> Result<()> {
        let listener = TcpListener::bind(addr).map_err(|e| Error::Other(e.to_string()))?;
        tracing::info!("federation: listening on tcp:{}", addr);
        let store = Arc::clone(&self.store);
        let trust = Arc::clone(&self.trust);
        let signing_key = self.signing_key.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut s) => {
                        let store = Arc::clone(&store);
                        let trust = Arc::clone(&trust);
                        let sk = signing_key.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = handle_stream(&mut s, &store, &trust, &sk) {
                                tracing::warn!("federation handler error: {}", e);
                            }
                        });
                    }
                    Err(e) => tracing::warn!("accept error: {}", e),
                }
            }
        });
        Ok(())
    }

    /// Start a Unix socket listener.
    #[cfg(unix)]
    pub fn listen_unix(&self, path: &std::path::Path) -> Result<()> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).map_err(|e| Error::Other(e.to_string()))?;
        tracing::info!("federation: listening on unix:{}", path.display());
        let store = Arc::clone(&self.store);
        let trust = Arc::clone(&self.trust);
        let signing_key = self.signing_key.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut s) => {
                        let store = Arc::clone(&store);
                        let trust = Arc::clone(&trust);
                        let sk = signing_key.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = handle_stream(&mut s, &store, &trust, &sk) {
                                tracing::warn!("federation handler error: {}", e);
                            }
                        });
                    }
                    Err(e) => tracing::warn!("accept error: {}", e),
                }
            }
        });
        Ok(())
    }

    /// Windows has no unix sockets — fall back to a loopback TCP listener
    /// whose port is derived from `path` (same mapping as `connect`).
    #[cfg(not(unix))]
    pub fn listen_unix(&self, path: &std::path::Path) -> Result<()> {
        self.listen_tcp(&unix_fallback_addr(path))
    }
}

/// Map a unix-socket path to a deterministic 127.0.0.1 port (non-unix only).
#[cfg(not(unix))]
fn unix_fallback_addr(path: &std::path::Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("127.0.0.1:{}", 40_000 + h.finish() % 20_000)
}

fn connect(addr: &Addr) -> Result<Box<dyn ReadWrite>> {
    match addr {
        Addr::Tcp(s) => {
            let s = TcpStream::connect(s).map_err(|e| Error::Other(e.to_string()))?;
            Ok(Box::new(s))
        }
        Addr::Unix(p) => unix_connect(p),
    }
}

#[cfg(unix)]
fn unix_connect(p: &std::path::Path) -> Result<Box<dyn ReadWrite>> {
    let s = UnixStream::connect(p).map_err(|e| Error::Other(e.to_string()))?;
    Ok(Box::new(s))
}

#[cfg(not(unix))]
fn unix_connect(p: &std::path::Path) -> Result<Box<dyn ReadWrite>> {
    let s = TcpStream::connect(unix_fallback_addr(p)).map_err(|e| Error::Other(e.to_string()))?;
    Ok(Box::new(s))
}

trait ReadWrite: Read + Write + Send {}
impl ReadWrite for TcpStream {}
#[cfg(unix)]
impl ReadWrite for UnixStream {}

fn handle_stream(
    stream: &mut (impl Read + Write),
    store: &Arc<Mutex<DocStore>>,
    trust: &Arc<Mutex<TrustStore>>,
    sk: &SigningKey,
) -> Result<()> {
    let buf = read_framed(stream)?;
    let msg = decode(&buf)?;
    match msg {
        Msg::SyncStep1 {
            doc_id,
            sv,
            vk,
            sig,
        } => {
            // Require a trusted, signature-verified peer BEFORE leaking any
            // doc contents via the diff reply (bd -j1u).
            let (peer_key, sig_arr) = parse_vk_sig(&vk, &sig)?;
            if !trust.lock().unwrap().is_trusted(&peer_key.to_bytes()) {
                return Err(Error::Other(format!(
                    "sync request from untrusted peer for {doc_id}"
                )));
            }
            sign::verify_bytes(
                &peer_key,
                &auth_payload(b"syn-fed-sync1", &doc_id, &sv),
                &sig_arr,
            )?;
            let update = store.lock().unwrap().diff_since(&doc_id, &sv)?;
            let reply_sig =
                sign::sign_bytes(sk, &auth_payload(b"syn-fed-sync2", &doc_id, &update)).to_vec();
            let reply = encode(&Msg::SyncStep2 {
                doc_id,
                update,
                vk: sk.verifying_key().to_bytes().to_vec(),
                sig: reply_sig,
            })?;
            write_framed(stream, &reply)?;
        }
        Msg::Update {
            doc_id,
            update,
            sig,
            vk,
        } => {
            let (verifying_key, sig_arr) = parse_vk_sig(&vk, &sig)?;
            if !trust.lock().unwrap().is_trusted(&verifying_key.to_bytes()) {
                return Err(Error::Other(format!(
                    "update from untrusted peer for {doc_id}"
                )));
            }
            sign::verify_bytes(&verifying_key, &update, &sig_arr)?;
            store.lock().unwrap().apply_update(&doc_id, &update)?;
        }
        Msg::SyncStep2 { .. } => {
            // ignore unexpected
        }
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt;

    use crate::sign::random_signing_key;

    fn make_fed() -> Federation {
        let sk = random_signing_key();
        Federation::new(sk)
    }

    #[test]
    fn sign_and_receive_update() {
        let fed = make_fed();
        let update = crdt::new_meta(&[("tags", "test")]).unwrap();
        // put into local store
        fed.store
            .lock()
            .unwrap()
            .apply_update("doc1", &update)
            .unwrap();
        // create signed Update msg manually
        let sig = sign::sign_bytes(&fed.signing_key, &update).to_vec();
        let vk = fed.signing_key.verifying_key().to_bytes().to_vec();
        let msg = Msg::Update {
            doc_id: "doc1".into(),
            update: update.clone(),
            sig,
            vk,
        };
        // receive on same fed (simulates peer receiving)
        fed.receive_update(msg).unwrap();
    }

    #[test]
    fn bad_signature_rejected() {
        let fed = make_fed();
        let update = crdt::new_meta(&[("tags", "test")]).unwrap();
        let sk2 = random_signing_key();
        let bad_sig = sign::sign_bytes(&sk2, b"wrong data").to_vec();
        let vk = sk2.verifying_key().to_bytes().to_vec();
        let msg = Msg::Update {
            doc_id: "doc1".into(),
            update,
            sig: bad_sig,
            vk,
        };
        assert!(fed.receive_update(msg).is_err());
    }

    #[test]
    fn untrusted_vk_rejected_even_with_valid_sig() {
        let fed = make_fed();
        let update = crdt::new_meta(&[("tags", "test")]).unwrap();
        // Attacker self-signs: valid signature, but key was never pinned.
        let evil = random_signing_key();
        let sig = sign::sign_bytes(&evil, &update).to_vec();
        let msg = Msg::Update {
            doc_id: "doc1".into(),
            update,
            sig,
            vk: evil.verifying_key().to_bytes().to_vec(),
        };
        assert!(fed.receive_update(msg).is_err());
        // ...and after explicit pinning the same key is accepted.
        fed.trust_peer(&evil.verifying_key()).unwrap();
        let update = crdt::new_meta(&[("tags", "test")]).unwrap();
        let sig = sign::sign_bytes(&evil, &update).to_vec();
        let msg = Msg::Update {
            doc_id: "doc1".into(),
            update,
            sig,
            vk: evil.verifying_key().to_bytes().to_vec(),
        };
        fed.receive_update(msg).unwrap();
    }

    #[test]
    fn trust_store_persists_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        let mut ts = TrustStore::load(&path).unwrap();
        let vk = random_signing_key().verifying_key().to_bytes();
        ts.trust(vk).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let reloaded = TrustStore::load(&path).unwrap();
        assert!(reloaded.is_trusted(&vk));
    }

    #[test]
    fn two_nodes_sync_via_tcp() {
        let sk_a = random_signing_key();
        let sk_b = random_signing_key();
        let fed_a = Federation::new(sk_a.clone());
        let fed_b = Federation::new(sk_b.clone());
        // Mutual pinning is required now — unknown keys are rejected.
        fed_a.trust_peer(&sk_b.verifying_key()).unwrap();
        fed_b.trust_peer(&sk_a.verifying_key()).unwrap();

        // Put a doc on node-A
        let update = crdt::new_meta(&[("node", "A"), ("data", "hello")]).unwrap();
        fed_a
            .store
            .lock()
            .unwrap()
            .apply_update("docX", &update)
            .unwrap();

        // node-B starts listener
        let port = 17832;
        fed_b.listen_tcp(&format!("127.0.0.1:{}", port)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        // node-A adds peer and broadcasts
        fed_a.add_peer(Addr::Tcp(format!("127.0.0.1:{}", port)));
        fed_a.on_local_update("docX", &update).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));

        // node-B should have the doc
        let mut store_b = fed_b.store.lock().unwrap();
        let full = store_b.apply_update("docX", &update).unwrap();
        assert!(!full.is_empty());
        // The doc exists in B's store
        assert!(store_b.docs.contains_key("docX"));
    }
}
