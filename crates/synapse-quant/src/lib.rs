//! synapse-quant — vector quantization primitives.
//!
//! Status: scaffolding (PR-0). See `docs/SCALE_100M_PLAN_2026-04-23.md` §2.3.
//!
//! Planned:
//!   * int8 linear-scale per-dim (PR-C1, ~2d) — 4× compression, <1% recall loss
//!   * Matryoshka 384→256 trunc+int8 (PR-C2) — 12× compression
//!   * Product Quantization (inside synapse-ann/ivfpq) — 32-64× compression

#![allow(dead_code)]

#[derive(Debug, thiserror::Error)]
pub enum QuantError {
    #[error("dim mismatch: expected {expected}, got {actual}")]
    DimMismatch { expected: usize, actual: usize },
    #[error("calibration required before encode")]
    NotCalibrated,
}

pub trait Quantizer {
    type Encoded;
    fn calibrate(&mut self, corpus: &[&[f32]]) -> Result<(), QuantError>;
    fn encode(&self, v: &[f32]) -> Result<Self::Encoded, QuantError>;
    fn decode(&self, e: &Self::Encoded) -> Result<Vec<f32>, QuantError>;
    /// Distance in encoded space — MUST be order-preserving w.r.t. decoded cosine/L2.
    fn distance(&self, a: &Self::Encoded, b: &Self::Encoded) -> f32;
}

pub mod int8;
pub use int8::{Int8, Int8Vec};

pub mod ivf;
pub use ivf::Ivf;

#[cfg(feature = "rabitq")]
pub mod rabitq;
#[cfg(feature = "rabitq")]
pub use rabitq::{RaBitQEncoder, RaBitQVec};
// TODO(PR-C2): pub mod matryoshka;
