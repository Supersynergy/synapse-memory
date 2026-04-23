//! Candle-Metal BGE-small embedder — scaffolding only.
//!
//! Status: **stub**. Wires the `TextEmbedder` trait contract so downstream code
//! can already route through a Metal backend once [`Self::embed_batch_real`]
//! is implemented. Loading a BGE-small model via `candle-transformers::bert`
//! is a 200-300 LOC addition tracked in the next PR.
//!
//! # Plan
//! 1. Load tokenizer from `~/.cache/huggingface/hub/models--BAAI--bge-small-en-v1.5`.
//! 2. Build `bert::BertModel` from `pytorch_model.bin` via `candle_nn::VarBuilder`.
//! 3. Forward + mean-pool + L2 normalize → `[n, 384]` on `Device::new_metal(0)`.
//!
//! Expected numbers on M4 Max (from `metal-candle` benchmarks): ~1-2 ms / doc,
//! batch 32 near-constant 4 ms.

use crate::error::{Error, Result};

/// Candle-Metal backed embedder for BGE-small.
#[derive(Debug)]
pub struct CandleMetalEmbedder {
    model_id: String,
    dim: usize,
    /// true once [`Self::embed_batch_real`] is wired.
    ready: bool,
}

impl CandleMetalEmbedder {
    /// Create a new embedder targeting a HuggingFace repo id.
    ///
    /// Only `"BAAI/bge-small-en-v1.5"` is supported for now (hard-coded dim 384).
    pub fn new(model_id: &str) -> Result<Self> {
        if !matches!(model_id, "BAAI/bge-small-en-v1.5" | "bge-small") {
            return Err(Error::Other(format!(
                "candle-metal: only bge-small supported in v2.1 preview, got {model_id}"
            )));
        }
        Ok(Self {
            model_id: "BAAI/bge-small-en-v1.5".to_string(),
            dim: 384,
            ready: false,
        })
    }

    /// Canonical batched path. Stub returns `Unimplemented` until PR-candle-metal lands.
    pub fn embed_batch(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if !self.ready {
            return Err(Error::Other(
                "candle-metal: inference path not yet implemented (see SPEC_V2 §4 Step E)".into(),
            ));
        }
        Ok(Vec::new())
    }

    /// Stable identifier for routing + metrics.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.model_id
    }

    /// Output dimensionality.
    #[must_use]
    pub const fn dim(&self) -> usize {
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_accepts_bge_small_alias() {
        assert!(CandleMetalEmbedder::new("bge-small").is_ok());
        assert!(CandleMetalEmbedder::new("BAAI/bge-small-en-v1.5").is_ok());
    }

    #[test]
    fn new_rejects_unknown_models() {
        assert!(CandleMetalEmbedder::new("all-MiniLM-L6-v2").is_err());
    }

    #[test]
    fn stub_batch_returns_unimplemented_error() {
        let e = CandleMetalEmbedder::new("bge-small").unwrap();
        assert!(e.embed_batch(&["hi".into()]).is_err());
    }

    #[test]
    fn dim_is_384_for_bge_small() {
        let e = CandleMetalEmbedder::new("bge-small").unwrap();
        assert_eq!(e.dim(), 384);
    }
}
