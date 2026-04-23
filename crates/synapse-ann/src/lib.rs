//! synapse-ann — pluggable ANN backends.
//!
//! Status: scaffolding only (PR-0 of scale-100M plan).
//! Do not use in production. See `docs/SCALE_100M_PLAN_2026-04-23.md`.
//!
//! Backends planned:
//!   * `ann-usearch` — usearch-rs binding (PR-A1, ~2d, drop-in for HNSW)
//!   * `ann-ivfpq`   — Rust-native IVF-PQ on faer kmeans (PR-A2, ~6-8d)

#![allow(dead_code)]

pub trait AnnIndex: Send + Sync {
    /// Insert a vector with an external id.
    fn insert(&mut self, id: u64, vector: &[f32]) -> Result<(), AnnError>;
    /// kNN search returning (id, distance) ascending by distance.
    fn search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>, AnnError>;
    /// Current number of inserted vectors.
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }
}

#[derive(Debug, thiserror::Error)]
pub enum AnnError {
    #[error("dim mismatch: expected {expected}, got {actual}")]
    DimMismatch { expected: usize, actual: usize },
    #[error("backend {0} not compiled in — enable the matching feature flag")]
    BackendDisabled(&'static str),
    #[error("backend error: {0}")]
    Other(String),
}

#[cfg(feature = "ann-usearch")]
pub mod usearch_backend;

#[cfg(feature = "ann-usearch")]
pub use usearch_backend::UsearchIndex;

// TODO(PR-A2): pub mod ivfpq;
