//! `.synx` reader — sequential read + manifest parse.
//!
//! v0.1 uses buffered reads. v0.2 will switch to mmap + zero-copy manifest.

use crate::error::{Error, Result};
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;

use super::chunk::Chunk;
use super::header::{FOOTER_SIZE, SynxFooter, SynxHeader};
use super::manifest::Manifest;

pub struct SynxReader {
    inner: BufReader<File>,
    pub header: SynxHeader,
    pub footer: SynxFooter,
    pub manifest: Manifest,
}

impl SynxReader {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref()).map_err(Error::from)?;
        let file_len = file.metadata().map_err(Error::from)?.len();
        let mut inner = BufReader::new(file);

        // Header
        inner.seek(SeekFrom::Start(0)).map_err(Error::from)?;
        let header = SynxHeader::read_from(&mut inner)?;

        // Footer at fixed tail offset
        if file_len < FOOTER_SIZE as u64 {
            return Err(Error::Format("file smaller than footer".into()));
        }
        inner
            .seek(SeekFrom::Start(file_len - FOOTER_SIZE as u64))
            .map_err(Error::from)?;
        let footer = SynxFooter::read_from(&mut inner)?;

        // Manifest chunk at header.manifest_offset
        inner
            .seek(SeekFrom::Start(header.manifest_offset))
            .map_err(Error::from)?;
        let manifest_chunk = Chunk::read_from(&mut inner)?;
        if manifest_chunk.hash != footer.manifest_hash {
            return Err(Error::Format("manifest hash does not match footer".into()));
        }
        let manifest_bytes = manifest_chunk.decode()?;
        let manifest = Manifest::from_json(&manifest_bytes)?;

        Ok(Self {
            inner,
            header,
            footer,
            manifest,
        })
    }

    /// Read a specific chunk by its manifest index.
    pub fn read_chunk_at(&mut self, idx: usize) -> Result<Chunk> {
        let r = self
            .manifest
            .chunks
            .get(idx)
            .ok_or_else(|| Error::Format(format!("chunk idx {} out of range", idx)))?;
        self.inner
            .seek(SeekFrom::Start(r.offset))
            .map_err(Error::from)?;
        Chunk::read_from(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::super::chunk::{ChunkKind, Codec};
    use super::super::header::SynxFlags;
    use super::super::writer::SynxWriter;
    use super::*;

    #[test]
    fn writer_reader_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.synx");

        let mut w = SynxWriter::create(&path, SynxFlags::COMPRESSED).unwrap();
        w.append(ChunkKind::TextBlob, Codec::Raw, b"hello synx")
            .unwrap();
        w.append(
            ChunkKind::RowBatch,
            Codec::Zstd,
            &b"bigger payload here ".repeat(50),
        )
        .unwrap();
        w.finish().unwrap();

        let mut r = SynxReader::open(&path).unwrap();
        assert_eq!(r.manifest.chunks.len(), 2);
        let c0 = r.read_chunk_at(0).unwrap();
        assert_eq!(c0.decode().unwrap(), b"hello synx");
        let c1 = r.read_chunk_at(1).unwrap();
        assert_eq!(c1.decode().unwrap().len(), 20 * 50);
    }
}
