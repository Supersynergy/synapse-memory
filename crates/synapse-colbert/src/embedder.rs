//! ColBERT embedder — placeholder dummy until candle-core/jina-colbert-v2 wired.
//! Produces deterministic pseudo-random 128-dim token vectors for smoke tests.
//! Replace `embed_doc` / `embed_query` bodies with real model inference.

use anyhow::Result;

pub const TOKEN_DIM: usize = 128;

/// Token-level multi-vector embedder.
/// Currently: dummy (hash-seeded random). Swap body for real candle inference.
pub struct ColbertEmbedder {
    /// Max tokens per document (ColBERT default 180; query default 32).
    pub max_doc_tokens: usize,
    pub max_query_tokens: usize,
}

impl Default for ColbertEmbedder {
    fn default() -> Self {
        Self { max_doc_tokens: 180, max_query_tokens: 32 }
    }
}

impl ColbertEmbedder {
    pub fn new(max_doc_tokens: usize, max_query_tokens: usize) -> Self {
        Self { max_doc_tokens, max_query_tokens }
    }

    /// Embed document → N token vectors, each TOKEN_DIM f32 (L2-normalised).
    pub fn embed_doc(&self, text: &str) -> Result<Vec<Vec<f32>>> {
        let n = (text.split_whitespace().count()).clamp(1, self.max_doc_tokens);
        Ok(dummy_embed(text, n, 0x44_6F_63u64))
    }

    /// Embed query → M token vectors (shorter, query expansion tokens included).
    pub fn embed_query(&self, text: &str) -> Result<Vec<Vec<f32>>> {
        let n = (text.split_whitespace().count()).clamp(1, self.max_query_tokens);
        Ok(dummy_embed(text, n, 0x51_72_79u64))
    }
}

/// Deterministic pseudo-random token vecs — replace with real model.
fn dummy_embed(seed_text: &str, n_tokens: usize, salt: u64) -> Vec<Vec<f32>> {
    let seed = seed_text.bytes().fold(salt, |acc, b| {
        acc.wrapping_mul(6364136223846793005u64).wrapping_add(b as u64).wrapping_add(1442695040888963407u64)
    });
    (0..n_tokens).map(|t| {
        let mut s = seed.wrapping_add((t as u64).wrapping_mul(0xDEAD_BEEF_CAFE_BABEu64));
        let raw: Vec<f32> = (0..TOKEN_DIM).map(|_| {
            s = s.wrapping_mul(6364136223846793005u64).wrapping_add(1442695040888963407u64);
            // map u64 → [-1, 1]
            (s as i64 as f32) / (i64::MAX as f32)
        }).collect();
        l2_norm(raw)
    }).collect()
}

fn l2_norm(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shapes() {
        let emb = ColbertEmbedder::default();
        let vecs = emb.embed_doc("hello world foo bar").unwrap();
        assert_eq!(vecs.len(), 4);
        assert_eq!(vecs[0].len(), TOKEN_DIM);
        // check normalised
        let norm: f32 = vecs[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "not normalised: {norm}");
    }
}
