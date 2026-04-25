//! MLX Metal embedder scaffold — Phase 5 Day 57-65.
//!
//! Real implementation (mlx-rs bindings, Metal kernel dispatch) is deferred.
//! This module compiles under the `embed-mlx` feature but all embed calls
//! return `unimplemented!` until Phase 5 Day 57 lands the actual dep.
//!
//! Enabled only on Apple Silicon macOS with feature `embed-mlx`.

#![cfg(all(target_os = "macos", target_arch = "aarch64", feature = "embed-mlx"))]

use crate::embedder_trait::TextEmbedder;
use crate::error::{Error, Result};

/// Embedding backend that targets the Apple Neural Engine / Metal GPU via MLX.
///
/// Fields are intentionally empty in the scaffold; Phase 5 Day 57 will add:
/// - `model: mlx_rs::Model`
/// - `tokenizer: tokenizers::Tokenizer`
/// - `dim: usize`
pub struct MlxMetalEmbedder {
    // TODO(Phase-5-Day-57): mlx_rs model handle
    _private: (),
}

impl MlxMetalEmbedder {
    /// Construct a new MLX Metal embedder.
    ///
    /// # Errors
    /// Always errors until Phase 5 Day 57 wires the real `mlx-rs` dep.
    pub fn new() -> Result<Self> {
        Err(Error::Other(
            "MLX backend not yet implemented — scheduled for Phase 5 Day 57-65".into(),
        ))
    }

    /// Model name as reported to the trait.
    pub fn backend_name(&self) -> &str {
        "mlx-metal:bge-small-en-v1.5"
    }

    /// Output dimensionality (BGE-small = 384).
    pub fn backend_dim(&self) -> usize {
        384
    }
}

impl TextEmbedder for MlxMetalEmbedder {
    fn name(&self) -> &str {
        self.backend_name()
    }

    fn dim(&self) -> usize {
        self.backend_dim()
    }

    fn embed_batch(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
        unimplemented!("MLX impl pending Phase 5 Day 57-65")
    }
}
