//! `.synx` header + footer binary layout.
//!
//! Header: 64 bytes at file offset 0.
//! Footer: 256 bytes at file end (fixed-size tail).

use crate::error::{Error, Result};
use std::io::{Read, Write};

pub const MAGIC: &[u8; 4] = b"SYNX";
pub const FOOTER_MAGIC: &[u8; 4] = b"XNYS";
pub const VERSION: u16 = 2;
pub const HEADER_SIZE: usize = 64;
pub const FOOTER_SIZE: usize = 256;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct SynxFlags: u16 {
        const COMPRESSED = 0b0000_0001;
        const SIGNED     = 0b0000_0010;
        const CRDT       = 0b0000_0100;
    }
}

#[derive(Clone, Debug)]
pub struct SynxHeader {
    pub version: u16,
    pub flags: SynxFlags,
    pub manifest_offset: u64,
    pub footer_offset: u64,
    pub created_unix: u64,
    pub creator_uuid: [u8; 16],
}

impl SynxHeader {
    pub fn new() -> Self {
        Self {
            version: VERSION,
            flags: SynxFlags::COMPRESSED,
            manifest_offset: 0,
            footer_offset: 0,
            created_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            creator_uuid: [0u8; 16],
        }
    }

    pub fn write_to<W: Write>(&self, w: &mut W) -> Result<()> {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(MAGIC);
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..8].copy_from_slice(&self.flags.bits().to_le_bytes());
        buf[8] = 0; // endian: little
        // 9..16 reserved (zero)
        buf[16..24].copy_from_slice(&self.manifest_offset.to_le_bytes());
        buf[24..32].copy_from_slice(&self.footer_offset.to_le_bytes());
        buf[32..40].copy_from_slice(&self.created_unix.to_le_bytes());
        buf[40..56].copy_from_slice(&self.creator_uuid);
        // 56..64 reserved
        w.write_all(&buf).map_err(Error::from)?;
        Ok(())
    }

    pub fn read_from<R: Read>(r: &mut R) -> Result<Self> {
        let mut buf = [0u8; HEADER_SIZE];
        r.read_exact(&mut buf).map_err(Error::from)?;
        if &buf[0..4] != MAGIC {
            return Err(Error::Format("bad magic — not a .synx file".into()));
        }
        let version = u16::from_le_bytes([buf[4], buf[5]]);
        if version != VERSION {
            return Err(Error::Format(format!(
                "unsupported version {} (expected {})",
                version, VERSION
            )));
        }
        let flags = SynxFlags::from_bits_truncate(u16::from_le_bytes([buf[6], buf[7]]));
        let manifest_offset = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let footer_offset = u64::from_le_bytes(buf[24..32].try_into().unwrap());
        let created_unix = u64::from_le_bytes(buf[32..40].try_into().unwrap());
        let mut creator_uuid = [0u8; 16];
        creator_uuid.copy_from_slice(&buf[40..56]);
        Ok(Self {
            version,
            flags,
            manifest_offset,
            footer_offset,
            created_unix,
            creator_uuid,
        })
    }
}

impl Default for SynxHeader {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct SynxFooter {
    pub manifest_hash: [u8; 32],
    pub signature: Option<[u8; 64]>,
    pub pubkey: Option<[u8; 32]>,
}

impl SynxFooter {
    pub fn write_to<W: Write>(&self, w: &mut W) -> Result<()> {
        let mut buf = [0u8; FOOTER_SIZE];
        buf[0..4].copy_from_slice(FOOTER_MAGIC);
        buf[4..36].copy_from_slice(&self.manifest_hash);
        if let (Some(sig), Some(pk)) = (&self.signature, &self.pubkey) {
            buf[36..100].copy_from_slice(sig);
            buf[100..132].copy_from_slice(pk);
        }
        // version tail at end
        buf[FOOTER_SIZE - 2..].copy_from_slice(&VERSION.to_le_bytes());
        w.write_all(&buf).map_err(Error::from)?;
        Ok(())
    }

    pub fn read_from<R: Read>(r: &mut R) -> Result<Self> {
        let mut buf = [0u8; FOOTER_SIZE];
        r.read_exact(&mut buf).map_err(Error::from)?;
        if &buf[0..4] != FOOTER_MAGIC {
            return Err(Error::Format("bad footer magic".into()));
        }
        let mut manifest_hash = [0u8; 32];
        manifest_hash.copy_from_slice(&buf[4..36]);
        // signature is optional; detect non-zero
        let signed = buf[36..100].iter().any(|&b| b != 0);
        let (signature, pubkey) = if signed {
            let mut sig = [0u8; 64];
            sig.copy_from_slice(&buf[36..100]);
            let mut pk = [0u8; 32];
            pk.copy_from_slice(&buf[100..132]);
            (Some(sig), Some(pk))
        } else {
            (None, None)
        };
        Ok(Self {
            manifest_hash,
            signature,
            pubkey,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn header_roundtrip() {
        let h = SynxHeader {
            version: VERSION,
            flags: SynxFlags::COMPRESSED | SynxFlags::CRDT,
            manifest_offset: 12345,
            footer_offset: 99999,
            created_unix: 1_700_000_000,
            creator_uuid: [1; 16],
        };
        let mut buf = Vec::new();
        h.write_to(&mut buf).unwrap();
        assert_eq!(buf.len(), HEADER_SIZE);
        let h2 = SynxHeader::read_from(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(h2.manifest_offset, 12345);
        assert_eq!(h2.footer_offset, 99999);
        assert!(h2.flags.contains(SynxFlags::CRDT));
    }

    #[test]
    fn footer_roundtrip_unsigned() {
        let f = SynxFooter {
            manifest_hash: [42; 32],
            signature: None,
            pubkey: None,
        };
        let mut buf = Vec::new();
        f.write_to(&mut buf).unwrap();
        assert_eq!(buf.len(), FOOTER_SIZE);
        let f2 = SynxFooter::read_from(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(f2.manifest_hash, [42; 32]);
        assert!(f2.signature.is_none());
    }
}
