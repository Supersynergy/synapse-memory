//! Auto-tiered index: RAM (MultiIndex) + optional disk-tier (SPANN).
//!
//! When `add` pushes the in-memory corpus past `SYNAPSE_RAM_THRESHOLD_DOCS`
//! (default 100_000, override via env), all subsequent docs spill to SPANN.
//! `search` queries both tiers and merges results by score.
//!
//! Feature-gated behind `spann-tier` — without the feature the SPANN path
//! is compiled out and the struct behaves as a plain incremental MultiIndex.
//!
//! ```rust,ignore
//! # #[cfg(feature = "spann-tier")] {
//! use synapse_core::turbo::tiered::TieredIndex;
//! let mut idx = TieredIndex::new(4, "/tmp/spann-test").unwrap();
//! for id in 0..200_000_i64 {
//!     idx.add(id, vec![id as f32, 0.0, 0.0, 0.0]).unwrap();
//! }
//! let hits = idx.search(&[1.0, 0.0, 0.0, 0.0], 10);
//! assert!(!hits.is_empty());
//! # }
//! ```

#![allow(clippy::type_complexity)]

use std::path::PathBuf;

use crate::turbo::multi_index::{MultiIndex, SearchHints};

#[cfg(feature = "spann-tier")]
use {
    anyhow::Result,
    synapse_spann::index::{SpannConfig, SpannIndex},
};

#[cfg(feature = "spann-tier")]
const DEFAULT_THRESHOLD: usize = 100_000;

#[cfg(feature = "spann-tier")]
fn ram_threshold() -> usize {
    std::env::var("SYNAPSE_RAM_THRESHOLD_DOCS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_THRESHOLD)
}

/// Pending docs buffered before the SPANN tier is built.
#[cfg(feature = "spann-tier")]
struct SpannBuffer {
    docs: Vec<(u64, Vec<f32>)>,
    dim: usize,
    dir: PathBuf,
    index: Option<SpannIndex>,
}

#[cfg(feature = "spann-tier")]
impl SpannBuffer {
    fn new(dim: usize, dir: PathBuf) -> Self {
        Self {
            docs: Vec::new(),
            dim,
            dir,
            index: None,
        }
    }

    fn add(&mut self, doc_id: u64, vec: Vec<f32>) -> Result<()> {
        self.docs.push((doc_id, vec));
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.docs.is_empty() {
            return Ok(());
        }
        let n = self.docs.len();
        let n_clusters = (n / 100).clamp(1, 4096);
        let cfg = SpannConfig {
            n_clusters,
            dim: self.dim,
            n_docs: n,
            max_iter: 50,
        };
        let spann_dir = self.dir.join("spann");
        let idx = SpannIndex::build(&spann_dir, &self.docs, cfg)?;
        self.index = Some(idx);
        // docs no longer needed after mmap
        self.docs.clear();
        self.docs.shrink_to_fit();
        Ok(())
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        let Some(ref idx) = self.index else {
            return vec![];
        };
        let nprobe = 8;
        idx.search(query, k, nprobe)
            .into_iter()
            .map(|(id, s)| (id as i64, s))
            .collect()
    }

    fn len(&self) -> usize {
        self.index.as_ref().map_or(0, |i| i.config.n_docs) + self.docs.len()
    }
}

/// Tiered ANN index — in-memory MultiIndex + optional SPANN disk tier.
pub struct TieredIndex {
    dim: usize,
    #[cfg(feature = "spann-tier")]
    threshold: usize,
    /// Buffered docs for the in-memory tier (built lazily on first search or explicit flush).
    mem_buf: Vec<(i64, Vec<f32>)>,
    mem: Option<MultiIndex>,
    #[cfg(feature = "spann-tier")]
    disk: SpannBuffer,
    #[cfg(not(feature = "spann-tier"))]
    _disk_dir: PathBuf,
}

impl TieredIndex {
    /// Create a new tiered index.
    ///
    /// * `dim`     — embedding dimension
    /// * `disk_dir`— base directory for SPANN files (`<disk_dir>/spann/`)
    #[cfg(feature = "spann-tier")]
    pub fn new(dim: usize, disk_dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let dir = disk_dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dim,
            threshold: ram_threshold(),
            mem_buf: Vec::new(),
            mem: None,
            disk: SpannBuffer::new(dim, dir),
        })
    }

    #[cfg(not(feature = "spann-tier"))]
    pub fn new(dim: usize, disk_dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        Ok(Self {
            dim,
            mem_buf: Vec::new(),
            mem: None,
            _disk_dir: disk_dir.into(),
        })
    }

    /// Add a document. Spills to disk tier when mem count exceeds threshold.
    #[cfg(feature = "spann-tier")]
    pub fn add(&mut self, doc_id: i64, vec: Vec<f32>) -> anyhow::Result<()> {
        assert_eq!(vec.len(), self.dim, "dim mismatch");
        if self.mem_count() < self.threshold {
            self.mem_buf.push((doc_id, vec));
            // Rebuild mem index lazily — invalidate stale index.
            self.mem = None;
        } else {
            // Ensure mem index is built before we start spilling.
            self.ensure_mem_built();
            self.disk.add(doc_id as u64, vec)?;
        }
        Ok(())
    }

    #[cfg(not(feature = "spann-tier"))]
    pub fn add(&mut self, doc_id: i64, vec: Vec<f32>) -> anyhow::Result<()> {
        assert_eq!(vec.len(), self.dim, "dim mismatch");
        self.mem_buf.push((doc_id, vec));
        self.mem = None;
        Ok(())
    }

    /// Force-flush the disk buffer (build SPANN index from accumulated docs).
    /// Called automatically on `search` when disk docs are pending.
    #[cfg(feature = "spann-tier")]
    pub fn flush_disk(&mut self) -> anyhow::Result<()> {
        self.disk.flush()
    }

    #[cfg(not(feature = "spann-tier"))]
    pub fn flush_disk(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn mem_count(&self) -> usize {
        self.mem_buf.len()
    }

    fn ensure_mem_built(&mut self) {
        if self.mem.is_none() && !self.mem_buf.is_empty() {
            self.mem = Some(MultiIndex::build(self.mem_buf.clone()));
        }
    }

    /// Search both tiers, merge by score (higher = better), return top-k.
    #[cfg(feature = "spann-tier")]
    pub fn search(&mut self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        self.ensure_mem_built();
        // Flush pending disk docs before searching.
        let _ = self.disk.flush();

        let hints = SearchHints {
            k,
            ..Default::default()
        };
        let mut results: Vec<(i64, f32)> = self
            .mem
            .as_ref()
            .map(|m| m.search(query, hints))
            .unwrap_or_default();

        let mut disk_hits = self.disk.search(query, k);
        results.append(&mut disk_hits);

        // Sort descending by score, keep top-k.
        results.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);
        results
    }

    #[cfg(not(feature = "spann-tier"))]
    pub fn search(&mut self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        self.ensure_mem_built();
        let hints = SearchHints {
            k,
            ..Default::default()
        };
        self.mem
            .as_ref()
            .map(|m| m.search(query, hints))
            .unwrap_or_default()
    }

    /// Total indexed docs across both tiers.
    #[cfg(feature = "spann-tier")]
    pub fn len(&self) -> usize {
        self.mem_count() + self.disk.len()
    }

    #[cfg(not(feature = "spann-tier"))]
    pub fn len(&self) -> usize {
        self.mem_count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_vec(id: i64, dim: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; dim];
        v[0] = id as f32;
        // L2-normalise so cosine similarity is well-defined.
        let norm = (v.iter().map(|x| x * x).sum::<f32>()).sqrt().max(1e-9);
        v.iter_mut().for_each(|x| *x /= norm);
        v
    }

    #[test]
    fn below_threshold_all_mem() {
        let dir = tempdir().unwrap();
        let mut idx = TieredIndex::new(4, dir.path()).unwrap();
        for id in 0..100_i64 {
            idx.add(id, make_vec(id, 4)).unwrap();
        }
        let hits = idx.search(&make_vec(42, 4), 5);
        assert!(!hits.is_empty());
        assert_eq!(idx.len(), 100);
    }

    #[cfg(feature = "spann-tier")]
    #[test]
    fn above_threshold_spills_to_disk() {
        let dir = tempdir().unwrap();
        // Use tiny threshold so we don't need 100k docs in a unit test.
        unsafe { std::env::set_var("SYNAPSE_RAM_THRESHOLD_DOCS", "50") };
        let mut idx = TieredIndex::new(4, dir.path()).unwrap();
        for id in 0..100_i64 {
            idx.add(id, make_vec(id, 4)).unwrap();
        }
        assert_eq!(idx.mem_count(), 50, "first 50 in mem");
        assert_eq!(idx.disk.len(), 50, "next 50 in disk buffer");
        let hits = idx.search(&make_vec(75, 4), 10);
        assert!(!hits.is_empty(), "disk-tier search returned results");
        // cleanup env override
        unsafe { std::env::remove_var("SYNAPSE_RAM_THRESHOLD_DOCS") };
    }
}
