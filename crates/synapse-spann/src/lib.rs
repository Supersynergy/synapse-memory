//! synapse-spann — disk-tier SPANN index (Microsoft 2021 concept).
//!
//! Architecture:
//!   - In-memory: centroid table (k-means, flat search for nprobe nearest)
//!   - On-disk:   per-cluster posting lists, mmap'd for zero-copy reads
//!   - Query:     find top-nprobe centroids → scan their posting lists → exact rerank
//!
//! Storage layout:
//!   <dir>/manifest.json          — n_clusters, n_docs, dim
//!   <dir>/centroids.bin          — k*dim f32 row-major
//!   <dir>/posting/<id>.bin       — N*(u64 docid + dim*f32) entries

pub mod build;
pub mod index;
pub mod posting;
pub mod search;

pub type DocId = u64;
pub type Embedding = Vec<f32>;
pub type DocumentEmbedding = (DocId, Embedding);
pub type SearchHit = (DocId, f32);
pub type SearchResults = Vec<SearchHit>;

pub use index::{SpannConfig, SpannIndex};
