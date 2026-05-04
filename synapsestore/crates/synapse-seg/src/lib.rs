//! synapse-seg — LSM-style segment store for vector partitions.
//!
//! Status: scaffolding (PR-0). See `docs/SCALE_100M_PLAN_2026-04-23.md` §2.2.
//!
//! Planned backend: fjall (pure Rust LSM). See plan §2.2 for evaluation.

#![allow(dead_code)]

#[derive(Debug, thiserror::Error)]
pub enum SegError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("tombstoned: {0}")]
    Tombstoned(u64),
}

// TODO(PR-B1): pub struct SegmentStore { ... } with insert / get / delete / compact
// TODO(PR-B2): size-tiered merge policy L0 → L1 → L2 → L3
