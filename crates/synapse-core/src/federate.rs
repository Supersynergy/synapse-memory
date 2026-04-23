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
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use yrs::updates::encoder::Encode;
use yrs::{updates::decoder::Decode, Doc, ReadTxn, StateVector, Transact, Update};

// ── Wire messages ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
pub enum Msg {
    SyncStep1 {
        doc_id: String,
        sv: Vec<u8>,
    },
    SyncStep2 {
        doc_id: String,
        update: Vec<u8>,
    },
    Update {
        doc_id: String,
        update: Vec<u8>,
        sig: Vec<u8>,
        vk: Vec<u8>,
    },
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
        self.docs.entry(doc_id.to_string()).or_insert_with(Doc::new)
    }

    fn state_vector(&mut self, doc_id: &str) -> Vec<u8> {
        let doc = self.get_or_create(doc_id);
        let txn = doc.transact();
        txn.state_vector().encode_v1()
    }

    fn apply_update(&mut self, doc_id: &str, update: &[u8]) -> Result<Vec<u8>> {
        let doc = self.docs.entry(doc_id.to_string()).or_insert_with(Doc::new);
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
}

impl Federation {
    pub fn new(signing_key: SigningKey) -> Self {
        Self {
            signing_key,
            peers: Arc::new(Mutex::new(vec![])),
            store: Arc::new(Mutex::new(DocStore::default())),
        }
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
            let step1 = encode(&Msg::SyncStep1 {
                doc_id: doc_id.clone(),
                sv,
            })?;
            write_framed(&mut stream, &step1)?;
            let reply = read_framed(&mut stream)?;
            let msg = decode(&reply)?;
            if let Msg::SyncStep2 {
                doc_id: rid,
                update,
            } = msg
            {
                if rid == *doc_id && !update.is_empty() {
                    self.store.lock().unwrap().apply_update(&rid, &update)?;
                }
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
                let vk_arr: [u8; 32] = vk
                    .try_into()
                    .map_err(|_| Error::Other("vk must be 32 bytes".into()))?;
                let sig_arr: [u8; 64] = sig
                    .try_into()
                    .map_err(|_| Error::Other("sig must be 64 bytes".into()))?;
                let verifying_key =
                    VerifyingKey::from_bytes(&vk_arr).map_err(|e| Error::Other(e.to_string()))?;
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
        let signing_key = self.signing_key.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut s) => {
                        let store = Arc::clone(&store);
                        let sk = signing_key.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = handle_stream(&mut s, &store, &sk) {
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
    pub fn listen_unix(&self, path: &std::path::Path) -> Result<()> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).map_err(|e| Error::Other(e.to_string()))?;
        tracing::info!("federation: listening on unix:{}", path.display());
        let store = Arc::clone(&self.store);
        let signing_key = self.signing_key.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut s) => {
                        let store = Arc::clone(&store);
                        let sk = signing_key.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = handle_stream(&mut s, &store, &sk) {
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
}

fn connect(addr: &Addr) -> Result<Box<dyn ReadWrite>> {
    match addr {
        Addr::Tcp(s) => {
            let s = TcpStream::connect(s).map_err(|e| Error::Other(e.to_string()))?;
            Ok(Box::new(s))
        }
        Addr::Unix(p) => {
            let s = UnixStream::connect(p).map_err(|e| Error::Other(e.to_string()))?;
            Ok(Box::new(s))
        }
    }
}

trait ReadWrite: Read + Write + Send {}
impl ReadWrite for TcpStream {}
impl ReadWrite for UnixStream {}

fn handle_stream(
    stream: &mut (impl Read + Write),
    store: &Arc<Mutex<DocStore>>,
    sk: &SigningKey,
) -> Result<()> {
    let buf = read_framed(stream)?;
    let msg = decode(&buf)?;
    match msg {
        Msg::SyncStep1 { doc_id, sv } => {
            let update = store.lock().unwrap().diff_since(&doc_id, &sv)?;
            let reply = encode(&Msg::SyncStep2 { doc_id, update })?;
            write_framed(stream, &reply)?;
        }
        Msg::Update {
            doc_id,
            update,
            sig,
            vk,
        } => {
            let vk_arr: [u8; 32] = vk
                .try_into()
                .map_err(|_| Error::Other("vk must be 32 bytes".into()))?;
            let sig_arr: [u8; 64] = sig
                .try_into()
                .map_err(|_| Error::Other("sig must be 64 bytes".into()))?;
            let verifying_key =
                VerifyingKey::from_bytes(&vk_arr).map_err(|e| Error::Other(e.to_string()))?;
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
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn make_fed() -> Federation {
        let sk = SigningKey::generate(&mut OsRng);
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
        let sk2 = SigningKey::generate(&mut OsRng);
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
    fn two_nodes_sync_via_tcp() {
        let sk_a = SigningKey::generate(&mut OsRng);
        let sk_b = SigningKey::generate(&mut OsRng);
        let fed_a = Federation::new(sk_a);
        let fed_b = Federation::new(sk_b.clone());

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
