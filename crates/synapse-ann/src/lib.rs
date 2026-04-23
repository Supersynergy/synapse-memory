//! synapse-ann — pluggable ANN backends.
//!
//! PR-A1 shipped: `UsearchIndex` behind `ann-usearch`, with insert/remove/search
//! + save/load/view for sidecar persistence. PR-A2 IVF-PQ still TODO.
//! See `docs/SCALE_100M_PLAN_2026-04-23.md`, SPEC §6.

#![allow(dead_code)]

use std::path::Path;

/// Minimal pluggable ANN surface. Add/search/remove + durable save.
pub trait AnnIndex: Send + Sync {
    /// Insert a vector with an external id.
    fn insert(&mut self, id: u64, vector: &[f32]) -> Result<(), AnnError>;

    /// Remove a previously-inserted id. Returns the count actually removed
    /// (0 if not present). Idempotent.
    fn remove(&mut self, id: u64) -> Result<usize, AnnError>;

    /// kNN search returning (id, distance) ascending by distance.
    fn search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>, AnnError>;

    /// Current number of inserted vectors.
    fn len(&self) -> usize;

    /// True when the index has no vectors.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Persist the index to `path` durably. Implementations should write
    /// atomically (tmp + rename) when practical.
    fn save(&self, path: &Path) -> Result<(), AnnError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AnnError {
    #[error("dim mismatch: expected {expected}, got {actual}")]
    DimMismatch { expected: usize, actual: usize },
    #[error("backend {0} not compiled in — enable the matching feature flag")]
    BackendDisabled(&'static str),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt index file: {0}")]
    Corrupt(String),
    #[error("version mismatch: file={file_version}, expected={expected_version}")]
    VersionMismatch {
        file_version: u32,
        expected_version: u32,
    },
    #[error("backend error: {0}")]
    Other(String),
}

#[cfg(feature = "ann-usearch")]
pub mod usearch_backend;

#[cfg(feature = "ann-usearch")]
pub use usearch_backend::UsearchIndex;

// TODO(PR-A2): pub mod ivfpq;
