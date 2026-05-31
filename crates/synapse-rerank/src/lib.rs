//! synapse-rerank: cross-encoder rerank stage for the SOTA recall pipeline.
//!
//! Two implementations:
//!  * `IdentityReranker` — no-op, always available, ~0ms (default build).
//!  * `OnnxCrossEncoder` — fastembed/ONNX-backed, feature `onnx`.
//!    Default model: `ms-marco-MiniLM-L-6-v2` (~22M params, ~2ms/pair on M-series).
//!
//! Stage signature: `rerank(query, candidates, top_k) -> Vec<Hit>`.
//! Cheaper signals already happened upstream (vec, FTS, RRF, heat).

use anyhow::Result;
use synapse_core::Hit;

pub mod cascade;
pub use cascade::CascadeReranker;

pub mod colbert;
pub use colbert::ColbertReranker;

pub mod factory;
pub use factory::build_reranker_from_env;

pub mod clicklog;

#[cfg(feature = "lightgbm")]
pub mod lightgbm;

pub trait Reranker: Send + Sync {
    fn rerank(&self, query: &str, candidates: Vec<Hit>, top_k: usize) -> Result<Vec<Hit>>;
}

/// Pass-through. Use when corpus is small (<200 docs) or budget is tight.
#[derive(Default)]
pub struct IdentityReranker;

impl Reranker for IdentityReranker {
    fn rerank(&self, _query: &str, mut candidates: Vec<Hit>, top_k: usize) -> Result<Vec<Hit>> {
        candidates.truncate(top_k);
        Ok(candidates)
    }
}

/// Score-blending helper: rerank score * 0.7 + original score * 0.3.
/// Keeps stable ordering when reranker is uncertain.
pub fn blend(rerank_score: f64, original_score: f64) -> f64 {
    0.7 * rerank_score + 0.3 * original_score
}

#[cfg(feature = "onnx")]
pub mod onnx {
    use super::*;
    use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
    use std::sync::Mutex;

    pub struct OnnxCrossEncoder {
        inner: Mutex<TextRerank>,
    }

    impl OnnxCrossEncoder {
        /// Default = BGE-reranker-v2-m3 (568M, multilingual, ~1ms/pair on M-series).
        /// More reliably cached than JINA v2 multilingual.
        /// Override with `from_model(RerankerModel::*)`.
        pub fn new() -> Result<Self> {
            Self::from_model(RerankerModel::BGERerankerV2M3)
        }

        /// JINA reranker v2 base multilingual (~140MB ONNX).
        /// Downloads only when fastembed cache is warm OR caller accepts the download.
        pub fn new_jina_v2() -> Result<Self> {
            Self::from_model(RerankerModel::JINARerankerV2BaseMultiligual)
        }

        pub fn from_model(model: RerankerModel) -> Result<Self> {
            let inner = TextRerank::try_new(RerankInitOptions::new(model))?;
            Ok(Self {
                inner: Mutex::new(inner),
            })
        }
    }

    impl Reranker for OnnxCrossEncoder {
        fn rerank(&self, query: &str, candidates: Vec<Hit>, top_k: usize) -> Result<Vec<Hit>> {
            if candidates.is_empty() {
                return Ok(candidates);
            }
            let docs: Vec<String> = candidates.iter().map(|h| h.text.clone()).collect();
            let docs_ref: Vec<&str> = docs.iter().map(|s| s.as_str()).collect();
            let mut guard = self.inner.lock().unwrap();
            let scored = guard.rerank(query, docs_ref, true, None)?;
            // scored is Vec<RerankResult { index, score, document }>
            let mut out: Vec<Hit> = scored
                .into_iter()
                .map(|r| {
                    let mut h = candidates[r.index].clone();
                    h.score = blend(r.score as f64, h.score);
                    h
                })
                .collect();
            out.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            out.truncate(top_k);
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(id: i64, score: f64, text: &str) -> Hit {
        Hit {
            id,
            uri: None,
            title: None,
            text: text.into(),
            score,
            meta: None,
            ts: None,
        }
    }

    #[test]
    fn identity_passthrough() {
        let r = IdentityReranker;
        let out = r
            .rerank("q", vec![h(1, 0.9, "a"), h(2, 0.5, "b")], 1)
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, 1);
    }

    #[test]
    fn blend_combines() {
        let s = blend(1.0, 0.0);
        assert!((s - 0.7).abs() < 1e-9);
    }
}
