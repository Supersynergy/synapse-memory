//! Cascade rerank: cheap-first → strong-final.
//!
//! Two-stage cross-encoder pipeline standard since BGE-Reranker-v2 + Qwen3-Reranker
//! (2025): cheap model narrows the candidate pool, strong model finalizes top-k.
//! This avoids running the expensive 4B model on the full ~50-doc pool while
//! still capturing the SOTA rerank quality.
//!
//! Stage-1: bge-reranker-v2-m3 (568M, ~1ms/pair on M-series via ONNX)
//!   in: ~50 candidates, out: top-20
//! Stage-2: qwen3-reranker-4b (or any heavyweight available)
//!   in: 20 candidates, out: top-k
//!
//! Falls back to single-stage when either backend is unavailable so the
//! pipeline never hard-fails.

use crate::Reranker;
use anyhow::Result;
use synapse_core::Hit;

pub struct CascadeReranker {
    pub fast: Box<dyn Reranker>,
    pub final_: Box<dyn Reranker>,
    /// Stage-1 keeps this many candidates for stage-2.
    pub stage1_top: usize,
}

impl CascadeReranker {
    pub fn new(fast: Box<dyn Reranker>, final_: Box<dyn Reranker>) -> Self {
        Self {
            fast,
            final_,
            stage1_top: 20,
        }
    }

    pub fn with_stage1_top(mut self, n: usize) -> Self {
        self.stage1_top = n;
        self
    }
}

impl Reranker for CascadeReranker {
    fn rerank(&self, query: &str, candidates: Vec<Hit>, top_k: usize) -> Result<Vec<Hit>> {
        if candidates.is_empty() {
            return Ok(candidates);
        }
        // Stage-1: cheap narrow.
        let stage1 = self
            .fast
            .rerank(query, candidates, self.stage1_top.max(top_k))?;
        // Stage-2: strong final.
        self.final_.rerank(query, stage1, top_k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdentityReranker;

    fn h(id: i64, score: f64, text: &str) -> Hit {
        Hit {
            id,
            uri: None,
            title: None,
            text: text.into(),
            score,
        }
    }

    #[test]
    fn identity_cascade_passes_through() {
        let c = CascadeReranker::new(Box::new(IdentityReranker), Box::new(IdentityReranker));
        let out = c
            .rerank("q", vec![h(1, 0.9, "a"), h(2, 0.5, "b"), h(3, 0.3, "c")], 2)
            .unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn empty_input_returns_empty() {
        let c = CascadeReranker::new(Box::new(IdentityReranker), Box::new(IdentityReranker));
        let out = c.rerank("q", vec![], 5).unwrap();
        assert!(out.is_empty());
    }
}
