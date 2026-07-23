//! `.synx` chunk framing.
//!
//! Each chunk is length-prefixed, kind-tagged, content-hashed, optionally zstd-compressed.

use crate::error::{Error, Result};
use std::io::{Read, Write};

/// 16 bytes of per-chunk framing header.
pub const CHUNK_HEADER_SIZE: usize = 16;

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkKind {
    RowBatch = 0x01,
    TextBlob = 0x02,
    FtsSegment = 0x03,
    VecIndex = 0x04,
    VecPayload = 0x05,
    CRDTOpsLog = 0x06,
    SchemaDef = 0x07,
    MerkleNode = 0x08,
    Tombstone = 0xFF,
}

impl ChunkKind {
    pub fn from_u16(v: u16) -> Result<Self> {
        Ok(match v {
            0x01 => Self::RowBatch,
            0x02 => Self::TextBlob,
            0x03 => Self::FtsSegment,
            0x04 => Self::VecIndex,
            0x05 => Self::VecPayload,
            0x06 => Self::CRDTOpsLog,
            0x07 => Self::SchemaDef,
            0x08 => Self::MerkleNode,
            0xFF => Self::Tombstone,
            _ => return Err(Error::Format(format!("unknown chunk kind {:#04x}", v))),
        })
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Raw = 0,
    Zstd = 1,
}

impl Codec {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Self::Raw,
            1 => Self::Zstd,
            _ => return Err(Error::Format(format!("unknown codec {}", v))),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Chunk {
    pub kind: ChunkKind,
    pub codec: Codec,
    pub flags: u8,
    pub hash: [u8; 32],
    pub uncompressed_len: u64,
    pub payload: Vec<u8>,
}

impl Chunk {
    /// Build a new chunk, hashing uncompressed bytes, optionally zstd-compressing.
    pub fn new(kind: ChunkKind, codec: Codec, data: &[u8]) -> Result<Self> {
        let hash = blake3::hash(data).into();
        let uncompressed_len = data.len() as u64;
        let payload = match codec {
            Codec::Raw => data.to_vec(),
            Codec::Zstd => zstd::stream::encode_all(data, 3).map_err(Error::from)?,
        };
        Ok(Self {
            kind,
            codec,
            flags: 0,
            hash,
            uncompressed_len,
            payload,
        })
    }

    /// Write the full framed chunk (header + hash + uncompressed_len + payload).
    pub fn write_to<W: Write>(&self, w: &mut W) -> Result<u64> {
        let mut hdr = [0u8; CHUNK_HEADER_SIZE];
        let total_len = (CHUNK_HEADER_SIZE + 32 + 8 + self.payload.len()) as u32;
        hdr[0..4].copy_from_slice(&total_len.to_le_bytes());
        hdr[4..6].copy_from_slice(&(self.kind as u16).to_le_bytes());
        hdr[6] = self.codec as u8;
        hdr[7] = self.flags;
        // 8..16 reserved
        w.write_all(&hdr).map_err(Error::from)?;
        w.write_all(&self.hash).map_err(Error::from)?;
        w.write_all(&self.uncompressed_len.to_le_bytes())
            .map_err(Error::from)?;
        w.write_all(&self.payload).map_err(Error::from)?;
        Ok(total_len as u64)
    }

    /// Read a framed chunk back.
    pub fn read_from<R: Read>(r: &mut R) -> Result<Self> {
        let mut hdr = [0u8; CHUNK_HEADER_SIZE];
        r.read_exact(&mut hdr).map_err(Error::from)?;
        let total_len = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
        let kind = ChunkKind::from_u16(u16::from_le_bytes([hdr[4], hdr[5]]))?;
        let codec = Codec::from_u8(hdr[6])?;
        let flags = hdr[7];
        let mut hash = [0u8; 32];
        r.read_exact(&mut hash).map_err(Error::from)?;
        let mut ulen_buf = [0u8; 8];
        r.read_exact(&mut ulen_buf).map_err(Error::from)?;
        let uncompressed_len = u64::from_le_bytes(ulen_buf);
        let payload_len = total_len
            .checked_sub(CHUNK_HEADER_SIZE + 32 + 8)
            .ok_or_else(|| Error::Format("chunk length underflow".into()))?;
        let mut payload = vec![0u8; payload_len];
        r.read_exact(&mut payload).map_err(Error::from)?;
        Ok(Self {
            kind,
            codec,
            flags,
            hash,
            uncompressed_len,
            payload,
        })
    }

    /// Decompress and verify hash against the uncompressed bytes.
    pub fn decode(&self) -> Result<Vec<u8>> {
        let data = match self.codec {
            Codec::Raw => self.payload.clone(),
            Codec::Zstd => zstd::stream::decode_all(&self.payload[..]).map_err(Error::from)?,
        };
        let h: [u8; 32] = blake3::hash(&data).into();
        if h != self.hash {
            return Err(Error::Format("chunk hash mismatch — corruption".into()));
        }
        if data.len() as u64 != self.uncompressed_len {
            return Err(Error::Format("uncompressed_len mismatch".into()));
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn chunk_roundtrip_raw() {
        let c = Chunk::new(ChunkKind::TextBlob, Codec::Raw, b"hello world").unwrap();
        let mut buf = Vec::new();
        c.write_to(&mut buf).unwrap();
        let c2 = Chunk::read_from(&mut Cursor::new(&buf)).unwrap();
        let decoded = c2.decode().unwrap();
        assert_eq!(decoded, b"hello world");
    }

    #[test]
    fn chunk_roundtrip_zstd() {
        let payload = b"the quick brown fox jumps over the lazy dog".repeat(100);
        let c = Chunk::new(ChunkKind::RowBatch, Codec::Zstd, &payload).unwrap();
        assert!(c.payload.len() < payload.len());
        let mut buf = Vec::new();
        c.write_to(&mut buf).unwrap();
        let c2 = Chunk::read_from(&mut Cursor::new(&buf)).unwrap();
        let decoded = c2.decode().unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn chunk_corruption_detected() {
        let mut c = Chunk::new(ChunkKind::TextBlob, Codec::Raw, b"data").unwrap();
        c.payload[0] ^= 0xFF;
        assert!(c.decode().is_err());
    }
}
