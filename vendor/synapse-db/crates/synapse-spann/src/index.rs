//! SpannIndex — public facade: build, save, load, search.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    DocumentEmbedding, SearchResults,
    build::{build_index, load_centroids},
    posting::MmapPostingList,
    search::{nearest_centroids, scan_and_rerank},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpannConfig {
    pub n_clusters: usize,
    pub dim: usize,
    pub n_docs: usize,
    /// k-means max iterations
    pub max_iter: u64,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    n_clusters: usize,
    n_docs: usize,
    dim: usize,
}

pub struct SpannIndex {
    pub config: SpannConfig,
    dir: PathBuf,
    /// In-memory centroid table
    centroids: Vec<Vec<f32>>,
    /// Mmap'd posting lists, indexed by cluster id
    posting_lists: Vec<MmapPostingList>,
}

impl SpannIndex {
    /// Build a new SPANN index from docs, persist to `dir`.
    pub fn build(dir: &Path, docs: &[DocumentEmbedding], config: SpannConfig) -> Result<Self> {
        fs::create_dir_all(dir)?;
        let centroids = build_index(dir, docs, config.n_clusters, config.dim, config.max_iter)?;
        let n_clusters = centroids.len();

        // Write manifest
        let manifest = Manifest {
            n_clusters,
            n_docs: docs.len(),
            dim: config.dim,
        };
        let mj = serde_json::to_string_pretty(&manifest)?;
        fs::write(dir.join("manifest.json"), mj)?;

        // Open posting lists
        let posting_lists = Self::open_posting_lists(dir, n_clusters, config.dim)?;

        Ok(Self {
            config: SpannConfig {
                n_clusters,
                n_docs: docs.len(),
                ..config
            },
            dir: dir.to_path_buf(),
            centroids,
            posting_lists,
        })
    }

    /// Load an existing SPANN index from `dir`.
    pub fn load(dir: &Path) -> Result<Self> {
        let mj = fs::read_to_string(dir.join("manifest.json"))?;
        let manifest: Manifest = serde_json::from_str(&mj)?;
        let centroids = load_centroids(
            &dir.join("centroids.bin"),
            manifest.n_clusters,
            manifest.dim,
        )?;
        let posting_lists = Self::open_posting_lists(dir, manifest.n_clusters, manifest.dim)?;
        Ok(Self {
            config: SpannConfig {
                n_clusters: manifest.n_clusters,
                dim: manifest.dim,
                n_docs: manifest.n_docs,
                max_iter: 100,
            },
            dir: dir.to_path_buf(),
            centroids,
            posting_lists,
        })
    }

    fn open_posting_lists(
        dir: &Path,
        n_clusters: usize,
        dim: usize,
    ) -> Result<Vec<MmapPostingList>> {
        let posting_dir = dir.join("posting");
        let mut lists = Vec::with_capacity(n_clusters);
        for i in 0..n_clusters {
            let p = posting_dir.join(format!("{i}.bin"));
            lists.push(MmapPostingList::open(&p, dim)?);
        }
        Ok(lists)
    }

    /// Search: top-nprobe centroids → scan posting lists → exact rerank → top-k.
    pub fn search(&self, query: &[f32], k: usize, nprobe: usize) -> SearchResults {
        let probe = nprobe.min(self.centroids.len());
        let cluster_ids = nearest_centroids(&self.centroids, query, probe);
        scan_and_rerank(&self.posting_lists, &cluster_ids, query, k)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}
