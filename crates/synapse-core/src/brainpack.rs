//! `.brainpack` v2 — distribution wrapper for `.synx` files.
//!
//! A brainpack is a `.synx` file that has been:
//!   1. finalised (manifest + footer written)
//!   2. optionally signed (Ed25519) via the `synx-v2` footer slot
//!   3. optionally wrapped in an additional zstd layer for smaller shipping size
//!
//! The on-disk format is either a bare `.synx` (magic `SYNX`) or a gzip-free
//! zstd stream whose decompressed body is a `.synx`. Readers detect both.
//!
//! Use `BrainPack::pack(synx_path, out)` to produce a shippable `.brainpack`,
//! and `BrainPack::unpack(path, out_synx)` to extract.

use crate::error::{Error, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use super::synx::header::MAGIC as SYNX_MAGIC;

pub const BRAINPACK_EXT: &str = "brainpack";

pub struct BrainPack;

impl BrainPack {
    /// Compress a `.synx` file into a shippable `.brainpack`.
    pub fn pack<P: AsRef<Path>, Q: AsRef<Path>>(synx: P, out: Q) -> Result<u64> {
        let mut input = File::open(synx.as_ref())?;
        let mut buf = Vec::new();
        input.read_to_end(&mut buf)?;
        if !buf.starts_with(SYNX_MAGIC) {
            return Err(Error::Format(
                "source is not a .synx file (bad magic)".into(),
            ));
        }
        let compressed = zstd::stream::encode_all(&buf[..], 19)
            .map_err(|e| Error::Format(format!("zstd encode failed: {e}")))?;
        let mut out_f = File::create(out.as_ref())?;
        out_f.write_all(&compressed)?;
        out_f.sync_all()?;
        Ok(compressed.len() as u64)
    }

    /// Extract a `.brainpack` back to a `.synx` file.
    pub fn unpack<P: AsRef<Path>, Q: AsRef<Path>>(pack: P, out: Q) -> Result<u64> {
        let mut input = File::open(pack.as_ref())?;
        let mut buf = Vec::new();
        input.read_to_end(&mut buf)?;
        // Two encodings: raw .synx (starts with SYNX magic) or zstd-wrapped.
        let data = if buf.starts_with(SYNX_MAGIC) {
            buf
        } else {
            zstd::stream::decode_all(&buf[..])
                .map_err(|e| Error::Format(format!("zstd decode failed: {e}")))?
        };
        if !data.starts_with(SYNX_MAGIC) {
            return Err(Error::Format(
                "unpacked body is not a .synx file (bad magic)".into(),
            ));
        }
        let mut out_f = File::create(out.as_ref())?;
        out_f.write_all(&data)?;
        out_f.sync_all()?;
        Ok(data.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::super::synx::{
        chunk::{ChunkKind, Codec},
        header::SynxFlags,
        reader::SynxReader,
        writer::SynxWriter,
    };
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let synx = dir.path().join("a.synx");
        let pack = dir.path().join("a.brainpack");
        let synx2 = dir.path().join("b.synx");

        let mut w = SynxWriter::create(&synx, SynxFlags::COMPRESSED).unwrap();
        for i in 0..20 {
            let t = format!("doc number {i}");
            w.append(ChunkKind::TextBlob, Codec::Zstd, t.as_bytes())
                .unwrap();
        }
        w.finish().unwrap();

        let orig = std::fs::metadata(&synx).unwrap().len();
        let packed = BrainPack::pack(&synx, &pack).unwrap();
        assert!(packed < orig);

        let unpacked = BrainPack::unpack(&pack, &synx2).unwrap();
        assert_eq!(unpacked, orig);

        let r = SynxReader::open(&synx2).unwrap();
        assert!(r.manifest.chunks.len() >= 20);
    }
}
