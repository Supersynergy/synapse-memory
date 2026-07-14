//! Posting-list codec: (DocId, Vec<f32>) entries, binary flat on disk, mmap'd on load.

use anyhow::Result;
use memmap2::Mmap;
use std::{fs, path::Path};

use crate::DocumentEmbedding;

/// Entry size in bytes: 8 (docid u64) + dim*4 (f32)
#[inline]
pub fn entry_bytes(dim: usize) -> usize {
    8 + dim * 4
}

/// Write a posting list for one cluster to disk.
/// Format: concatenated entries — each `[u64 docid][f32 x dim]` little-endian.
pub fn write_posting(path: &Path, entries: &[DocumentEmbedding], dim: usize) -> Result<()> {
    if entries.is_empty() {
        // write empty file
        fs::write(path, [])?;
        return Ok(());
    }
    let entry_sz = entry_bytes(dim);
    let mut buf = vec![0u8; entries.len() * entry_sz];
    for (i, (docid, vec)) in entries.iter().enumerate() {
        let off = i * entry_sz;
        buf[off..off + 8].copy_from_slice(&docid.to_le_bytes());
        for (j, v) in vec.iter().enumerate() {
            let start = off + 8 + j * 4;
            buf[start..start + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    fs::write(path, buf)?;
    Ok(())
}

/// Mmap-backed posting list.
pub struct MmapPostingList {
    mmap: Option<Mmap>,
    dim: usize,
}

impl MmapPostingList {
    pub fn open(path: &Path, dim: usize) -> Result<Self> {
        let meta = fs::metadata(path)?;
        if meta.len() == 0 {
            return Ok(Self { mmap: None, dim });
        }
        let file = fs::File::open(path)?;
        // SAFETY: file is read-only; process must not truncate while mmap is live.
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(Self {
            mmap: Some(mmap),
            dim,
        })
    }

    pub fn len(&self) -> usize {
        match &self.mmap {
            None => 0,
            Some(m) => m.len() / entry_bytes(self.dim),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate decoded entries (docid, slice of f32).
    /// Returns owned vecs to avoid lifetime entanglement.
    pub fn entries(&self) -> Vec<DocumentEmbedding> {
        let mmap = match &self.mmap {
            None => return vec![],
            Some(m) => m,
        };
        let esz = entry_bytes(self.dim);
        let n = mmap.len() / esz;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let off = i * esz;
            let docid = u64::from_le_bytes(mmap[off..off + 8].try_into().unwrap());
            let mut vec = Vec::with_capacity(self.dim);
            for j in 0..self.dim {
                let s = off + 8 + j * 4;
                vec.push(f32::from_le_bytes(mmap[s..s + 4].try_into().unwrap()));
            }
            out.push((docid, vec));
        }
        out
    }
}
