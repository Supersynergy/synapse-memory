//! Online learner module — first embedded columnar store with online-learner state
//! co-located in page-sidecar — restart latency 0ms, learner trains incrementally per tick.
//!
//! # Architecture
//!
//! Each `Series` carries a map of named `OnlineLearner` instances.  State is persisted
//! as a `.lnr` sidecar file beside the `.smx` data file.  Format:
//! `[name_len: u32 LE][name: utf8][bytes_len: u32 LE][learner_bytes]*`
//!
//! Supported implementations:
//! - [`FtrlLearner`] — FTRL-Proximal logistic regression, O(d) per tick, p50 < 500 ns.

pub mod ftrl;

pub use ftrl::FtrlLearner;

/// Per-tick online learner trait.
pub trait OnlineLearner: Send + Sync {
    /// Update model with one labeled sample. Returns log-loss on this sample.
    fn update(&mut self, features: &[f32], y: f32) -> f32;
    /// Predict probability in [0, 1].
    fn predict(&self, features: &[f32]) -> f32;
    /// Serialize state to bytes (for sidecar persistence).
    fn serialize(&self) -> Vec<u8>;
    /// Deserialize from bytes. Returns `None` on format mismatch.
    fn deserialize_from(bytes: &[u8]) -> Option<Self>
    where
        Self: Sized;
}
