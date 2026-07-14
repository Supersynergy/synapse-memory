//! `.synx` writer — append-only chunk writer, manifest + footer on flush.

use crate::error::{Error, Result};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use super::chunk::{Chunk, ChunkKind, Codec};
use super::header::{HEADER_SIZE, SynxFlags, SynxFooter, SynxHeader};
use super::manifest::Manifest;

pub struct SynxWriter {
    inner: BufWriter<File>,
    header: SynxHeader,
    manifest: Manifest,
    offset: u64,
}

impl SynxWriter {
    /// Create a new empty `.synx` file.
    pub fn create<P: AsRef<Path>>(path: P, flags: SynxFlags) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .read(true)
            .open(path.as_ref())
            .map_err(Error::from)?;
        let mut w = BufWriter::new(file);
        let mut header = SynxHeader::new();
        header.flags = flags;
        header.write_to(&mut w)?;
        Ok(Self {
            inner: w,
            header,
            manifest: Manifest::default(),
            offset: HEADER_SIZE as u64,
        })
    }

    /// Append a pre-built chunk and record it in the manifest.
    pub fn write_chunk(&mut self, chunk: &Chunk) -> Result<()> {
        let start = self.offset;
        let len = chunk.write_to(&mut self.inner)?;
        self.manifest.add_chunk(chunk.kind, start, len, chunk.hash);
        self.offset += len;
        Ok(())
    }

    /// Convenience: build + write from raw bytes.
    pub fn append(&mut self, kind: ChunkKind, codec: Codec, data: &[u8]) -> Result<()> {
        let c = Chunk::new(kind, codec, data)?;
        self.write_chunk(&c)
    }

    /// Finalize: write manifest + footer, flush.
    pub fn finish(mut self) -> Result<()> {
        // Manifest chunk at end of data region
        let manifest_bytes = self.manifest.to_json()?;
        let manifest_offset = self.offset;
        let mc = Chunk::new(ChunkKind::SchemaDef, Codec::Zstd, &manifest_bytes)?;
        let manifest_hash = mc.hash;
        let len = mc.write_to(&mut self.inner)?;
        self.offset += len;

        // Footer
        let footer_offset = self.offset;
        let footer = SynxFooter {
            manifest_hash,
            signature: None,
            pubkey: None,
        };
        footer.write_to(&mut self.inner)?;

        // Backpatch header offsets.
        self.inner.flush().map_err(Error::from)?;
        let mut file = self
            .inner
            .into_inner()
            .map_err(|e| Error::Format(format!("buffer flush failed: {e}")))?;
        self.header.manifest_offset = manifest_offset;
        self.header.footer_offset = footer_offset;
        file.seek(SeekFrom::Start(0)).map_err(Error::from)?;
        self.header.write_to(&mut file)?;
        file.sync_all().map_err(Error::from)?;
        Ok(())
    }
}
