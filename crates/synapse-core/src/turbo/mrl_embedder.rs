//! MRL-wrapping embedder decorator.
//!
//! Composes any [`TextEmbedder`] with a Matryoshka truncation step: the first
//! `k` dimensions of every output vector are kept and re-L2-normalized.
//! For MRL-trained models (BGE-M3, e5-mrl, OpenAI-3, bge-base-en-v1.5 with MRL
//! head) this yields a 3-6× matvec speed-up with near-full recall.
//!
//! # Example
//! ```no_run
//! # use synapse_core::embedder_trait::{TextEmbedder, build_embedder};
//! # use synapse_core::turbo::mrl_embedder::MrlEmbedder;
//! let base = build_embedder(None)?;
//! let mrl  = MrlEmbedder::new(base, 128);
//! let v    = mrl.embed_one("hello")?;
//! assert_eq!(v.len(), 128);
//! # Ok::<_, synapse_core::error::Error>(())
//! ```

use crate::embedder_trait::TextEmbedder;
use crate::error::Result;
use crate::matryoshka::truncate_row;

/// MRL-wrapping embedder decorator.
pub struct MrlEmbedder {
    inner: Box<dyn TextEmbedder>,
    k: usize,
    name: String,
}

impl MrlEmbedder {
    /// Wrap `inner`, keeping the first `k` dims of every output vector.
    ///
    /// Panics if `k == 0` or `k > inner.dim()`.
    #[must_use]
    pub fn new(inner: Box<dyn TextEmbedder>, k: usize) -> Self {
        assert!(k > 0, "MRL k must be > 0");
        assert!(k <= inner.dim(), "MRL k {} exceeds inner dim {}", k, inner.dim());
        let name = format!("{}@mrl{k}", inner.name());
        Self { inner, k, name }
    }

    /// Truncation target dimensionality.
    #[must_use]
    pub const fn k(&self) -> usize {
        self.k
    }
}

impl TextEmbedder for MrlEmbedder {
    fn name(&self) -> &str {
        &self.name
    }
    fn dim(&self) -> usize {
        self.k
    }
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let full = self.inner.embed_batch(texts)?;
        Ok(full.into_iter().map(|v| truncate_row(&v, self.k)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockEmbedder {
        dim: usize,
    }
    impl TextEmbedder for MockEmbedder {
        fn name(&self) -> &str { "mock" }
        fn dim(&self) -> usize { self.dim }
        fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            // deterministic: each output is [1, 2, 3, ..., dim]
            Ok(texts
                .iter()
                .map(|_| (1..=self.dim).map(|i| i as f32).collect())
                .collect())
        }
    }

    #[test]
    fn mrl_truncates_dim() {
        let inner: Box<dyn TextEmbedder> = Box::new(MockEmbedder { dim: 384 });
        let mrl = MrlEmbedder::new(inner, 128);
        assert_eq!(mrl.dim(), 128);
        let v = mrl.embed_one("t").unwrap();
        assert_eq!(v.len(), 128);
    }

    #[test]
    fn mrl_renormalizes() {
        let inner: Box<dyn TextEmbedder> = Box::new(MockEmbedder { dim: 8 });
        let mrl = MrlEmbedder::new(inner, 4);
        let v = mrl.embed_one("t").unwrap();
        let nrm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((nrm - 1.0).abs() < 1e-5, "got norm {nrm}");
    }

    #[test]
    fn mrl_name_tagged() {
        let inner: Box<dyn TextEmbedder> = Box::new(MockEmbedder { dim: 16 });
        let mrl = MrlEmbedder::new(inner, 8);
        assert_eq!(mrl.name(), "mock@mrl8");
    }

    #[test]
    #[should_panic(expected = "MRL k must be > 0")]
    fn mrl_k_zero_panics() {
        let inner: Box<dyn TextEmbedder> = Box::new(MockEmbedder { dim: 8 });
        let _ = MrlEmbedder::new(inner, 0);
    }

    #[test]
    #[should_panic(expected = "MRL k 32 exceeds inner dim 8")]
    fn mrl_k_gt_inner_panics() {
        let inner: Box<dyn TextEmbedder> = Box::new(MockEmbedder { dim: 8 });
        let _ = MrlEmbedder::new(inner, 32);
    }
}
