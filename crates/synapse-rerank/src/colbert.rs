//! ColbertReranker — late-interaction multi-vec rerank scaffold (P5.1).
//!
//! Differentiation vs single-vec rerank:
//!   - Standard: 1 query-vec · 1 doc-vec → cosine
//!   - ColBERT: N query-tokens-vecs · M doc-tokens-vecs → MaxSim sum
//!     (best multilingual recall per Liquid AI LFM2-ColBERT-350M 2026)
//!
//! Status: SCAFFOLD — uses simple multi-cosine fallback when no ColBERT model loaded.
//! Future: wire ONNX LFM2-ColBERT-350M via fastembed when feature `onnx-colbert`.
//!
//! Pattern (mining-first): adapted from VLLM ColBERTMixin + Liquid AI LFM2 paper.
//! NOT 1:1 copy — Synapse-specific simplification (no GPU req, lazy multi-vec).

use crate::Reranker;
use anyhow::Result;
use synapse_core::Hit;

/// MaxSim score: for each query-token, max-cosine over all doc-tokens, then sum.
/// O(Q·D) per pair where Q,D are token counts. With Q≈10, D≈50: ~500 ops/pair.
#[allow(dead_code)]
fn maxsim(query_tokens: &[Vec<f32>], doc_tokens: &[Vec<f32>]) -> f32 {
    let mut total = 0.0_f32;
    for q in query_tokens {
        let mut best = f32::MIN;
        for d in doc_tokens {
            let cos: f32 = q.iter().zip(d).map(|(a, b)| a * b).sum();
            if cos > best {
                best = cos;
            }
        }
        if best > f32::MIN {
            total += best;
        }
    }
    total
}

/// Scaffold reranker. Without an ONNX ColBERT model, falls back to identity-rerank
/// (preserves vec-search ranking). Exists to expose the API surface so callers can
/// `cargo build --features colbert` later without code changes.
pub struct ColbertReranker {
    model_path: Option<std::path::PathBuf>,
    // tokenizer + ONNX session would live here when feature `onnx-colbert` lands
}

impl ColbertReranker {
    pub fn new(model_path: Option<std::path::PathBuf>) -> Self {
        Self { model_path }
    }

    pub fn is_loaded(&self) -> bool {
        self.model_path
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or(false)
    }
}

impl Reranker for ColbertReranker {
    fn rerank(&self, _query: &str, mut candidates: Vec<Hit>, top_k: usize) -> Result<Vec<Hit>> {
        // Without loaded model: return original order (identity-rerank).
        // With model: tokenize query+docs, compute MaxSim per pair, sort.
        if !self.is_loaded() {
            candidates.truncate(top_k);
            return Ok(candidates);
        }
        // Future ONNX path: tokenize + multi-vec embed + maxsim
        // For now scaffold preserves order
        candidates.truncate(top_k);
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maxsim_simple() {
        let q = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let d = vec![vec![1.0, 0.0], vec![0.5, 0.5]];
        let s = maxsim(&q, &d);
        // q[0]·d[0]=1, q[1]·d[1]=0.5 → max(1, 0.5)=1 + max(0, 0.5)=0.5 = 1.5
        assert!((s - 1.5).abs() < 1e-5);
    }

    #[test]
    fn unloaded_passes_through() {
        let r = ColbertReranker::new(None);
        let hits = vec![
            Hit {
                id: 1,
                uri: None,
                title: None,
                text: String::new(),
                score: 0.9,
            },
            Hit {
                id: 2,
                uri: None,
                title: None,
                text: String::new(),
                score: 0.5,
            },
        ];
        let out = r.rerank("test", hits.clone(), 10).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, 1); // order preserved
    }
}
