//! Memory-mapped reader for `.synx` — Phase 3 Track (b).
//!
//! Opens a file with `memmap2::Mmap`, parses the fixed header directly from
//! the mapped bytes, resolves the manifest chunk without buffered I/O, and
//! hands out zero-copy slices for each chunk payload (caller decodes).
//!
//! The JSON manifest is decoded once on open; rkyv archival is Phase-3.2.

#[cfg(feature = "synx-mmap")]
pub use imp::*;
#[cfg(not(feature = "synx-mmap"))]
pub use stub::*;

#[cfg(feature = "synx-mmap")]
mod imp {
    use crate::error::{Error, Result};
    use crate::synx::chunk::{Chunk, ChunkKind, Codec};
    use crate::synx::header::{FOOTER_SIZE, HEADER_SIZE, MAGIC, SynxFlags, SynxFooter, SynxHeader};
    use crate::synx::manifest::{ChunkRef, Manifest};
    use memmap2::Mmap;
    use std::fs::File;
    use std::path::Path;

    pub struct MmapReader {
        _file: File,
        map: Mmap,
        pub header: SynxHeader,
        pub footer: SynxFooter,
        pub manifest: Manifest,
    }

    impl MmapReader {
        pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
            let file = File::open(path.as_ref())?;
            let map = unsafe { Mmap::map(&file)? };
            if map.len() < HEADER_SIZE + FOOTER_SIZE {
                return Err(Error::Format("file smaller than header+footer".into()));
            }
            if &map[0..4] != MAGIC {
                return Err(Error::Format("not a .synx file".into()));
            }
            // parse header from mapped slice
            let mut hdr_cursor = std::io::Cursor::new(&map[0..HEADER_SIZE]);
            let header = SynxHeader::read_from(&mut hdr_cursor)?;
            // footer at fixed tail
            let ft_off = map.len() - FOOTER_SIZE;
            let mut ft_cursor = std::io::Cursor::new(&map[ft_off..]);
            let footer = SynxFooter::read_from(&mut ft_cursor)?;
            // manifest chunk
            let off = header.manifest_offset as usize;
            if off + 16 > map.len() {
                return Err(Error::Format("manifest offset past EOF".into()));
            }
            let mut mf_cursor = std::io::Cursor::new(&map[off..]);
            let manifest_chunk = Chunk::read_from(&mut mf_cursor)?;
            if manifest_chunk.hash != footer.manifest_hash {
                return Err(Error::Format("manifest hash mismatch".into()));
            }
            let bytes = manifest_chunk.decode()?;
            let manifest = Manifest::from_json(&bytes)?;
            Ok(Self {
                _file: file,
                map,
                header,
                footer,
                manifest,
            })
        }

        /// Zero-copy slice of a payload (includes the chunk framing header).
        pub fn raw_slice(&self, idx: usize) -> Result<&[u8]> {
            let r: &ChunkRef = self
                .manifest
                .chunks
                .get(idx)
                .ok_or_else(|| Error::Format(format!("idx {idx} out of range")))?;
            let start = r.offset as usize;
            let end = start + r.length as usize;
            if end > self.map.len() {
                return Err(Error::Format("chunk end past EOF".into()));
            }
            Ok(&self.map[start..end])
        }

        /// Decode a chunk at index with the standard chunk parser.
        pub fn read_chunk(&self, idx: usize) -> Result<Chunk> {
            let slice = self.raw_slice(idx)?;
            let mut c = std::io::Cursor::new(slice);
            Chunk::read_from(&mut c)
        }

        pub fn is_signed(&self) -> bool {
            self.header.flags.contains(SynxFlags::SIGNED)
        }

        pub fn chunk_count_by_kind(&self, kind: ChunkKind) -> usize {
            self.manifest
                .chunks
                .iter()
                .filter(|r| r.kind_raw == kind as u16)
                .count()
        }

        /// Cheap hint for callers that need to decide between zstd-expand and raw.
        pub fn codec_hint(&self, idx: usize) -> Result<Codec> {
            let slice = self.raw_slice(idx)?;
            if slice.len() < 16 {
                return Err(Error::Format("chunk too small for hint".into()));
            }
            Codec::from_u8(slice[6])
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::synx::chunk::ChunkKind as Kind;
        use crate::synx::chunk::Codec as Cod;
        use crate::synx::writer::SynxWriter;

        #[test]
        fn mmap_reads_manifest_and_chunks() {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("mm.synx");
            let mut w = SynxWriter::create(&p, SynxFlags::COMPRESSED).unwrap();
            for i in 0..50 {
                let s = format!("chunk {i}");
                w.append(Kind::TextBlob, Cod::Zstd, s.as_bytes()).unwrap();
            }
            w.finish().unwrap();
            let r = MmapReader::open(&p).unwrap();
            assert_eq!(r.chunk_count_by_kind(Kind::TextBlob), 50);
            let c = r.read_chunk(0).unwrap();
            assert_eq!(c.decode().unwrap(), b"chunk 0");
            assert!(!r.is_signed());
        }
    }
}

#[cfg(not(feature = "synx-mmap"))]
mod stub {
    use crate::error::{Error, Result};
    use std::path::Path;

    pub struct MmapReader;
    impl MmapReader {
        pub fn open<P: AsRef<Path>>(_p: P) -> Result<Self> {
            Err(Error::Format(
                "synx-mmap feature disabled — rebuild with --features synx-mmap".into(),
            ))
        }
    }
}
