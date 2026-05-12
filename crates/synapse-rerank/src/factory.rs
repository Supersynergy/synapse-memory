use crate::{IdentityReranker, Reranker};

/// Build a reranker from the `SYNAPSE_RERANKER` environment variable.
///
/// Accepted values:
///   `identity`           — pass-through (default when env is unset or empty)
///   `lightgbm:/path/to/model.lgb` — LightGBM gradient-boosted reranker (feature `lightgbm`)
///   `onnx`               — ONNX cross-encoder (feature `onnx`)
///
/// Falls back to `IdentityReranker` on any error (missing feature, bad path, load failure).
pub fn build_reranker_from_env() -> Box<dyn Reranker> {
    let spec = std::env::var("SYNAPSE_RERANKER").unwrap_or_default();
    let spec = spec.trim();

    if spec.is_empty() || spec == "identity" {
        return Box::new(IdentityReranker);
    }

    #[cfg(feature = "lightgbm")]
    if let Some(path_str) = spec.strip_prefix("lightgbm:") {
        let path = std::path::Path::new(path_str);
        match crate::lightgbm::LightGbmReranker::load(path) {
            Ok(r) => {
                tracing::info!("reranker: LightGBM from {path_str}");
                return Box::new(r);
            }
            Err(e) => {
                tracing::warn!("LightGbmReranker load failed ({e}) — falling back to IdentityReranker");
                return Box::new(IdentityReranker);
            }
        }
    }

    #[cfg(feature = "onnx")]
    if spec == "onnx" {
        match crate::onnx::OnnxCrossEncoder::new() {
            Ok(r) => {
                tracing::info!("reranker: OnnxCrossEncoder (BGE-reranker-v2-m3)");
                return Box::new(r);
            }
            Err(e) => {
                tracing::warn!("OnnxCrossEncoder init failed ({e}) — falling back to IdentityReranker");
                return Box::new(IdentityReranker);
            }
        }
    }

    tracing::warn!("SYNAPSE_RERANKER={spec:?} unrecognised or feature not compiled in — using IdentityReranker");
    Box::new(IdentityReranker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_default_is_identity() {
        // Ensure env is unset for this test.
        std::env::remove_var("SYNAPSE_RERANKER");
        let r = build_reranker_from_env();
        // Should succeed as IdentityReranker.
        let out = r.rerank("q", vec![], 10).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn factory_identity_explicit() {
        std::env::set_var("SYNAPSE_RERANKER", "identity");
        let r = build_reranker_from_env();
        let out = r.rerank("q", vec![], 5).unwrap();
        assert!(out.is_empty());
        std::env::remove_var("SYNAPSE_RERANKER");
    }

    #[test]
    fn factory_lightgbm_missing_falls_back() {
        std::env::set_var("SYNAPSE_RERANKER", "lightgbm:/nonexistent/path/model.lgb");
        let r = build_reranker_from_env();
        // Must not panic, must return something usable (IdentityReranker fallback).
        let out = r.rerank("q", vec![], 5).unwrap();
        assert!(out.is_empty());
        std::env::remove_var("SYNAPSE_RERANKER");
    }

    #[test]
    fn factory_unknown_falls_back() {
        std::env::set_var("SYNAPSE_RERANKER", "bogus_value");
        let r = build_reranker_from_env();
        let out = r.rerank("q", vec![], 5).unwrap();
        assert!(out.is_empty());
        std::env::remove_var("SYNAPSE_RERANKER");
    }
}
