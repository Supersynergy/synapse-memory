//! Unified `TextEmbedder` trait — single seam for fastembed, Ollama, and future MLX/ANE backends.
//!
//! # Design
//!
//! Every embedder exposes:
//! * [`TextEmbedder::dim`] — output vector dimensionality (fixed per model).
//! * [`TextEmbedder::name`] — stable identifier for cache-keying & metrics.
//! * [`TextEmbedder::embed_batch`] — canonical batched path.
//! * [`TextEmbedder::embed_one`] — convenience default delegating to `embed_batch`.
//!
//! Selection at runtime is driven by [`EmbedderKind`] + [`build_embedder`], which
//! honours the `SYNAPSE_EMBEDDER` env var (`fastembed` | `ollama` | default = `fastembed`).
//!
//! # Example
//! ```no_run
//! use synapse_core::embedder_trait::{TextEmbedder, build_embedder};
//! let emb = build_embedder(None)?;                   // picks from env
//! let v = emb.embed_one("hello")?;                    // 384-d for BGE-small / minilm
//! # Ok::<_, synapse_core::error::Error>(())
//! ```

use crate::error::{Error, Result};

#[cfg(feature = "ollama")]
use crate::turbo::ollama_embedder;
use crate::turbo::candle_metal_embedder;

/// Backend selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedderKind {
    /// fastembed-rs ONNX, CPU. Default, no external deps.
    Fastembed,
    /// Ollama HTTP daemon — 17× faster than fastembed on M4 Max when running.
    Ollama,
    /// Candle-Metal BGE-small (Apple Silicon GPU) — scaffold only in v2.1.
    CandleMetal,
}

impl EmbedderKind {
    /// Parse from lowercase string; `None` falls back to default.
    #[must_use]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "fastembed" | "" => Some(Self::Fastembed),
            "ollama"          => Some(Self::Ollama),
            "candle-metal" | "candle" | "metal" => Some(Self::CandleMetal),
            _                 => None,
        }
    }

    /// Read `SYNAPSE_EMBEDDER` env var. Falls back to `Fastembed`.
    #[must_use]
    pub fn from_env() -> Self {
        std::env::var("SYNAPSE_EMBEDDER")
            .ok()
            .and_then(|v| Self::from_str(&v))
            .unwrap_or(Self::Fastembed)
    }
}

/// Object-safe embedder contract.
pub trait TextEmbedder: Send + Sync {
    /// Stable identifier, e.g. `"fastembed:bge-small-en-v1.5"`.
    fn name(&self) -> &str;

    /// Output dimensionality.
    fn dim(&self) -> usize;

    /// Canonical path: embed a slice of texts, return row-major vectors.
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Convenience: single-text embed. Default impl re-uses [`Self::embed_batch`].
    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut out = self.embed_batch(&[text.to_string()])?;
        out.pop()
            .ok_or_else(|| Error::Other("empty embed result".into()))
    }
}

// --- fastembed adapter ---------------------------------------------------

#[cfg(feature = "embed")]
impl TextEmbedder for crate::embed::Embedder {
    fn name(&self) -> &str { "fastembed:bge-small-en-v1.5" }
    fn dim(&self) -> usize { 384 }
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        crate::embed::Embedder::embed_batch(self, texts)
    }
}

// --- ollama adapter ------------------------------------------------------

impl TextEmbedder for candle_metal_embedder::CandleMetalEmbedder {
    fn name(&self) -> &str { candle_metal_embedder::CandleMetalEmbedder::name(self) }
    fn dim(&self) -> usize { candle_metal_embedder::CandleMetalEmbedder::dim(self) }
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        candle_metal_embedder::CandleMetalEmbedder::embed_batch(self, texts)
    }
}

#[cfg(feature = "ollama")]
impl TextEmbedder for ollama_embedder::OllamaEmbedder {
    fn name(&self) -> &str { "ollama:all-minilm" }
    fn dim(&self) -> usize { 384 }
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        ollama_embedder::OllamaEmbedder::embed_batch(self, texts)
    }
}

/// Build an embedder from an explicit kind, or from `SYNAPSE_EMBEDDER` env var.
///
/// # Errors
/// Returns `Error::Other` when the requested backend cannot be initialised
/// (model download failure, missing Ollama daemon, unavailable feature flag).
pub fn build_embedder(kind: Option<EmbedderKind>) -> Result<Box<dyn TextEmbedder>> {
    let k = kind.unwrap_or_else(EmbedderKind::from_env);
    match k {
        EmbedderKind::Fastembed => {
            #[cfg(feature = "embed")]
            {
                let e = crate::embed::Embedder::new()?;
                Ok(Box::new(e))
            }
            #[cfg(not(feature = "embed"))]
            {
                Err(Error::Other(
                    "fastembed backend requested but crate built without `embed` feature".into(),
                ))
            }
        }
        EmbedderKind::Ollama => {
            #[cfg(feature = "ollama")]
            {
                let e = ollama_embedder::OllamaEmbedder::new("all-minilm")?;
                Ok(Box::new(e))
            }
            #[cfg(not(feature = "ollama"))]
            {
                Err(Error::Other(
                    "ollama backend requested but crate built without `ollama` feature".into(),
                ))
            }
        }
        EmbedderKind::CandleMetal => {
            let e = candle_metal_embedder::CandleMetalEmbedder::new("bge-small")?;
            Ok(Box::new(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_from_str_parses_known_variants() {
        assert_eq!(EmbedderKind::from_str("fastembed"), Some(EmbedderKind::Fastembed));
        assert_eq!(EmbedderKind::from_str("OLLAMA"),    Some(EmbedderKind::Ollama));
        assert_eq!(EmbedderKind::from_str("bogus"),     None);
    }

    #[test]
    fn kind_from_env_defaults_to_fastembed() {
        // Don't depend on ambient env; exercise the parse path directly.
        assert_eq!(EmbedderKind::from_str(""),  Some(EmbedderKind::Fastembed));
    }
}
