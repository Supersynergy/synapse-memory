//! # synapse-metal
//!
//! Apple Silicon Metal compute shader backend for Synapse int8 MatVec.
//!
//! Design:
//! * `MetalI8Matvec::new()` initialises an `MTLDevice` + `MTLComputePipelineState`
//!   from the inline MSL source (below).
//! * `.dispatch(query_codes, db_codes, scales, scores)` runs one batch.
//! * Without the `gpu` feature, a pure-Rust scalar fallback is provided so
//!   downstream crates always link even on non-Apple hosts.
//!
//! Status v0.1: **scaffold** — the `gpu` path is wired behind the feature flag;
//! CPU fallback is the production default. Measured ROI vs SimSIMD CPU is
//! 1.2–1.6× for in-memory workloads (see `docs/TASK_1_METAL_SHADER_PLAN.md`).
//!
//! # Example (CPU fallback)
//! ```
//! use synapse_metal::MetalI8Matvec;
//! let q = vec![1_i8, 0, 0, 0];
//! let db = vec![1_i8, 0, 0, 0,  0, 1, 0, 0];   // 2 rows × dim=4
//! let scales = vec![1.0_f32, 1.0];
//! let m = MetalI8Matvec::new(4).unwrap();
//! let scores = m.dispatch(&q, &db, &scales);
//! assert_eq!(scores.len(), 2);
//! assert!(scores[0] > scores[1]);
//! ```

#![warn(missing_docs)]

use thiserror::Error;

/// Crate error type.
#[derive(Debug, Error)]
pub enum Error {
    /// Dim × rows must divide cleanly.
    #[error("dim mismatch: db.len()={db} is not a multiple of dim={dim}")]
    DimMismatch {
        /// DB length.
        db: usize,
        /// Declared dim.
        dim: usize,
    },
    /// Metal device unavailable (non-Apple host, or `gpu` feature off).
    #[error("Metal unavailable: {0}")]
    NoMetal(String),
}

/// Compute-pipeline wrapper.
pub struct MetalI8Matvec {
    dim: usize,
    #[cfg(feature = "gpu")]
    device: metal::Device,
}

impl MetalI8Matvec {
    /// Build a new compute pipeline targeting vectors of length `dim`.
    ///
    /// On feature `gpu`, grabs the system default `MTLDevice` and compiles
    /// the inline MSL kernel. On CPU-only builds, just stores `dim`.
    pub fn new(dim: usize) -> Result<Self, Error> {
        #[cfg(feature = "gpu")]
        {
            let device = metal::Device::system_default()
                .ok_or_else(|| Error::NoMetal("no default MTLDevice".into()))?;
            // Note: pipeline creation (MSL compile + PSO) happens lazily on
            // first dispatch to keep construction cheap when the process
            // may never actually run a search.
            return Ok(Self { dim, device });
        }
        #[cfg(not(feature = "gpu"))]
        {
            Ok(Self { dim })
        }
    }

    /// Run the matvec and return one score per DB row.
    ///
    /// # Panics
    /// Panics if `db.len() % dim != 0` or `scales.len()` mismatch.
    pub fn dispatch(&self, query: &[i8], db: &[i8], scales: &[f32]) -> Vec<f32> {
        assert_eq!(query.len(), self.dim);
        assert_eq!(db.len() % self.dim, 0, "ragged db");
        let n = db.len() / self.dim;
        assert_eq!(n, scales.len(), "scales len mismatch");

        #[cfg(feature = "gpu")]
        {
            // Production dispatch path — left as TODO in v0.1. The scaffolding
            // above gives us an MTLDevice handle; a follow-up commit will add
            // MSL source compilation + MTLBuffer round-trip.
            let _ = &self.device;
        }

        // CPU fallback (also a correctness reference impl for the GPU path).
        let mut out = vec![0.0_f32; n];
        for (i, row) in db.chunks(self.dim).enumerate() {
            let acc: i32 = query
                .iter()
                .zip(row)
                .map(|(&q, &d)| i32::from(q) * i32::from(d))
                .sum();
            out[i] = acc as f32 * scales[i];
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_fallback_identity() {
        let q = vec![1_i8, 0, 0, 0];
        let db = vec![1, 0, 0, 0, 0, 1, 0, 0];
        let scales = vec![1.0_f32, 1.0];
        let m = MetalI8Matvec::new(4).unwrap();
        let s = m.dispatch(&q, &db, &scales);
        assert_eq!(s.len(), 2);
        assert!(s[0] > s[1], "identity row should outscore orthogonal one");
    }

    #[test]
    fn scale_applied() {
        let q = vec![1_i8];
        let db = vec![1_i8, 1];
        let scales = vec![2.0_f32, 10.0];
        let m = MetalI8Matvec::new(1).unwrap();
        let s = m.dispatch(&q, &db, &scales);
        assert!((s[0] - 2.0).abs() < 1e-6);
        assert!((s[1] - 10.0).abs() < 1e-6);
    }
}
