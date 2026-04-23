//! Wire protocol: length-prefixed msgpack over unix socket.
//! Frame: [u32 LE length][msgpack body]

use serde::{Deserialize, Serialize};
use synapsedb_core::{Hit, PutRequest, SearchMode};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", content = "args")]
pub enum Request {
    Ping,
    Put(PutReq),
    PutBatch(Vec<PutReq>),
    Search {
        mode: SearchMode,
        q: String,
        limit: usize,
        embed_query: bool,
    },
    Stats,
    Snap {
        out: String,
        level: i32,
    },
    Shutdown,
    /// Merge CRDT state into a doc. `state` is base64-encoded yrs update bytes.
    Merge {
        id: i64,
        state: Vec<u8>,
    },
    /// Return docs ordered by timestamp descending.
    Timeline {
        limit: usize,
        offset: usize,
    },
    /// Verify Ed25519 sig on a doc. `vk` is 32-byte raw verifying key.
    Verify {
        id: i64,
        vk: Vec<u8>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutReq {
    pub title: Option<String>,
    pub uri: Option<String>,
    pub text: String,
    pub meta: Option<serde_json::Value>,
    /// If true, embed server-side before insert.
    pub embed: bool,
}

impl From<PutReq> for PutRequest {
    fn from(p: PutReq) -> Self {
        PutRequest {
            title: p.title,
            uri: p.uri,
            text: p.text,
            meta: p.meta,
            embedding: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Pong,
    Id(i64),
    Ids(Vec<i64>),
    Hits(Vec<Hit>),
    Docs(Vec<synapsedb_core::Doc>),
    Stats { docs: i64, vecs: i64 },
    Ok,
    Err(String),
}
