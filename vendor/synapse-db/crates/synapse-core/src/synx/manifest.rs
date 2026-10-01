//! `.synx` manifest — chunk index, schema history, snapshot cursor.
//!
//! v0.1 uses JSON for simplicity; v0.2 will migrate to rkyv zero-copy archival.

use serde::{Deserialize, Serialize};

use super::chunk::ChunkKind;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkRef {
    pub kind_raw: u16,
    pub offset: u64,
    pub length: u64,
    pub hash: [u8; 32],
}

impl ChunkRef {
    pub fn new(kind: ChunkKind, offset: u64, length: u64, hash: [u8; 32]) -> Self {
        Self {
            kind_raw: kind as u16,
            offset,
            length,
            hash,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub version: u32,
    pub arrow_schema_json: String,
    pub created_unix: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ManifestStats {
    pub n_docs: u64,
    pub n_vectors: u64,
    pub n_segments: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub chunks: Vec<ChunkRef>,
    pub schema_history: Vec<SchemaVersion>,
    pub active_snapshot: u64,
    pub stats: ManifestStats,
    pub merkle_root: [u8; 32],
}

impl Manifest {
    pub fn to_json(&self) -> Result<Vec<u8>, crate::error::Error> {
        serde_json::to_vec(self).map_err(|e| crate::error::Error::Format(e.to_string()))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, crate::error::Error> {
        serde_json::from_slice(bytes).map_err(|e| crate::error::Error::Format(e.to_string()))
    }

    pub fn add_chunk(&mut self, kind: ChunkKind, offset: u64, length: u64, hash: [u8; 32]) {
        self.chunks.push(ChunkRef::new(kind, offset, length, hash));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_json_roundtrip() {
        let mut m = Manifest::default();
        m.add_chunk(ChunkKind::RowBatch, 64, 1024, [7; 32]);
        m.stats.n_docs = 1000;
        let js = m.to_json().unwrap();
        let m2 = Manifest::from_json(&js).unwrap();
        assert_eq!(m2.chunks.len(), 1);
        assert_eq!(m2.stats.n_docs, 1000);
    }
}
