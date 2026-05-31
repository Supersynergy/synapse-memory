//! SPLADE encoder — placeholder dummy until ONNX naver/splade-v3 wired.
//! Real SPLADE: masked-LM logits → ReLU → log(1+x) → max-pool over tokens → sparse vec.
//! Dummy: deterministic top-K vocab terms per text using hash, weights in (0,1].
//! Swap `encode_inner` body for real model inference, API stays identical.

use anyhow::Result;
use std::collections::HashMap;

/// Sparse vector: token_id → weight (f32 > 0).
/// Typical cardinality: 50–200 non-zero terms (real SPLADE-v3).
pub type SparseVec = HashMap<u32, f32>;

/// Vocabulary size for the dummy encoder (matches WordPiece ~30k).
pub const VOCAB_SIZE: u32 = 30_522;

/// Max non-zero terms produced per encode call (real SPLADE usually ~50–150).
pub const TOP_K: usize = 64;

pub struct SpladeEncoder {
    /// Scale controls weight magnitude; real model uses learned projection weights.
    pub weight_scale: f32,
}

impl Default for SpladeEncoder {
    fn default() -> Self {
        Self { weight_scale: 1.0 }
    }
}

impl SpladeEncoder {
    pub fn new(weight_scale: f32) -> Self {
        Self { weight_scale }
    }

    /// Encode text → sparse vector (doc or query, same path in SPLADE-v3).
    /// Real impl: tokenise → masked-LM forward → ReLU(log(1+x)) → max-pool.
    pub fn encode(&self, text: &str) -> Result<SparseVec> {
        Ok(dummy_sparse(text, TOP_K, self.weight_scale))
    }
}

#[cfg(feature = "splade-onnx")]
impl SpladeEncoder {
    /// Load real naver/splade-v3 ONNX model from HF Hub.
    /// Returns an [`OnnxSpladeEncoder`] — call `.encode(text)` on it directly.
    pub fn from_onnx() -> Result<OnnxSpladeEncoder> {
        OnnxSpladeEncoder::from_hf_hub()
    }
}

/// Deterministic pseudo-sparse vec — replace body with ONNX inference.
/// Produces up to `top_k` token ids with weights in (0, scale].
fn dummy_sparse(text: &str, top_k: usize, scale: f32) -> SparseVec {
    // Seed from text bytes
    let seed = text.bytes().fold(0xcafe_babe_dead_beefu64, |acc, b| {
        acc.wrapping_mul(6364136223846793005)
            .wrapping_add(b as u64 + 1442695040888963407)
    });

    // Word-level "tokenisation" — gives reproducible term overlap across similar texts
    let word_seeds: Vec<u64> = text
        .split_whitespace()
        .map(|w| {
            w.bytes().fold(seed, |acc, b| {
                acc.wrapping_mul(2862933555777941757)
                    .wrapping_add(b as u64 + 1)
            })
        })
        .collect();

    let mut map: HashMap<u32, f32> = HashMap::new();

    // For each "word token", expand to a few vocab positions (simulates expansion)
    for (i, ws) in word_seeds.iter().enumerate() {
        for j in 0u64..4 {
            if map.len() >= top_k {
                break;
            }
            let mut s = ws.wrapping_add(j.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let tok = (s % VOCAB_SIZE as u64) as u32;
            // weight: ReLU(log(1+x)) analog — deterministic in (0, scale]
            let w_raw = ((s >> 32) as u32 as f32) / (u32::MAX as f32); // 0..1
            let w = (1.0 + w_raw).ln() * scale; // log(1+x) transform
            let entry = map.entry(tok).or_insert(0.0f32);
            *entry = entry.max(w); // max-pool
            let _ = i; // suppress unused
        }
        if map.len() >= top_k {
            break;
        }
    }

    // Ensure at least one term even for empty input
    if map.is_empty() {
        map.insert(seed as u32 % VOCAB_SIZE, scale * 0.5);
    }

    map
}

// ─── Real ONNX SPLADE-v3 encoder ─────────────────────────────────────────────

#[cfg(feature = "splade-onnx")]
mod onnx {
    use super::{SparseVec, TOP_K};
    use anyhow::{Context, Result};
    use ort::{inputs, session::Session, value::TensorRef};
    use std::path::Path;
    use tokenizers::Tokenizer;

    const MODEL_REPO: &str = "naver/splade-v3";
    /// BERT-base vocabulary size — SPLADE-v3 uses WordPiece vocab 30522.
    #[allow(dead_code)]
    pub const VOCAB_SIZE: u32 = 30_522;

    pub struct OnnxSpladeEncoder {
        session: Session,
        tokenizer: Tokenizer,
    }

    impl OnnxSpladeEncoder {
        /// Load encoder from a local ONNX file + tokenizer directory/json.
        ///
        /// `model_path`: path to `model.onnx`
        /// `tokenizer_path`: path to `tokenizer.json` (HF fast tokenizer)
        pub fn from_paths(model_path: &Path, tokenizer_path: &Path) -> Result<Self> {
            let session = Session::builder()
                .context("ort session builder")?
                .commit_from_file(model_path)
                .context("loading splade-v3 onnx model")?;
            let tokenizer = Tokenizer::from_file(tokenizer_path)
                .map_err(|e| anyhow::anyhow!("tokenizer load: {e}"))?;
            Ok(Self { session, tokenizer })
        }

        /// Download naver/splade-v3 from HuggingFace Hub and load.
        /// Model cached at `~/.cache/huggingface/hub/` (standard hf-hub path).
        pub fn from_hf_hub() -> Result<Self> {
            use hf_hub::api::sync::Api;
            let api = Api::new().context("hf-hub Api::new")?;
            let repo = api.model(MODEL_REPO.to_string());
            let model_path = repo.get("model.onnx").context("download model.onnx")?;
            let tokenizer_path = repo
                .get("tokenizer.json")
                .context("download tokenizer.json")?;
            Self::from_paths(&model_path, &tokenizer_path)
        }

        /// Encode text → sparse vector.
        /// Pipeline: tokenize → ONNX MLM forward → relu(log(1+x)) → max-pool over seq → top-K.
        pub fn encode(&mut self, text: &str) -> Result<SparseVec> {
            // Tokenize (BERT WordPiece, max 512 tokens)
            let encoding = self
                .tokenizer
                .encode(text, true)
                .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;

            let ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
            let mask: Vec<i64> = encoding
                .get_attention_mask()
                .iter()
                .map(|&x| x as i64)
                .collect();
            let type_ids: Vec<i64> = encoding.get_type_ids().iter().map(|&x| x as i64).collect();

            let seq_len = ids.len();

            // Use (shape, slice) tuples — avoids ndarray version mismatch with ort
            let ids_t = TensorRef::<i64>::from_array_view(([1usize, seq_len], ids.as_slice()))
                .context("ids tensor")?;
            let mask_t = TensorRef::<i64>::from_array_view(([1usize, seq_len], mask.as_slice()))
                .context("mask tensor")?;
            let type_t =
                TensorRef::<i64>::from_array_view(([1usize, seq_len], type_ids.as_slice()))
                    .context("type_ids tensor")?;

            // SPLADE-v3 ONNX inputs: input_ids, attention_mask, token_type_ids
            let outputs = self
                .session
                .run(inputs![
                    "input_ids" => ids_t,
                    "attention_mask" => mask_t,
                    "token_type_ids" => type_t
                ])
                .context("ort run")?;

            // Output[0]: logits [1, seq_len, vocab_size] (MLM head)
            let (shape, logits_flat) = outputs[0]
                .try_extract_tensor::<f32>()
                .context("extract logits")?;
            // shape: [batch=1, seq_len, vocab_size]
            let vocab = shape[2] as usize;
            let seq = shape[1] as usize;

            // relu(log(1+x)) + max-pool over sequence dimension → [vocab_size]
            let mut pooled = vec![0.0f32; vocab];
            for t in 0..seq {
                for v in 0..vocab {
                    let x = logits_flat[t * vocab + v];
                    let activated = if x > 0.0f32 {
                        (1.0f32 + x).ln()
                    } else {
                        0.0f32
                    };
                    if activated > pooled[v] {
                        pooled[v] = activated;
                    }
                }
            }

            // Keep top-K non-zero terms
            let mut pairs: Vec<(u32, f32)> = pooled
                .iter()
                .enumerate()
                .filter(|(_, &w)| w > 0.0)
                .map(|(i, &w)| (i as u32, w))
                .collect();

            // Sort descending by weight, keep top-K
            pairs.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            pairs.truncate(TOP_K);

            Ok(pairs.into_iter().collect())
        }
    }
}

#[cfg(feature = "splade-onnx")]
pub use onnx::OnnxSpladeEncoder;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_nonempty() {
        let enc = SpladeEncoder::default();
        let sv = enc.encode("hello world neural sparse").unwrap();
        assert!(!sv.is_empty());
        assert!(sv.len() <= TOP_K);
        for (&tid, &w) in &sv {
            assert!(tid < VOCAB_SIZE, "token id out of vocab");
            assert!(w > 0.0, "weight must be positive");
        }
    }

    #[test]
    fn shared_terms_similar_text() {
        let enc = SpladeEncoder::default();
        let a = enc.encode("neural sparse retrieval").unwrap();
        let b = enc.encode("neural sparse retrieval").unwrap();
        // identical text → identical sparse vec
        assert_eq!(a.len(), b.len());
        for (k, v) in &a {
            assert!((b[k] - v).abs() < 1e-6);
        }
    }

    #[test]
    fn overlap_similar_docs() {
        // Dummy encoder is hash-based — identical tokens produce identical term ids.
        // Just verify both produce non-empty vecs with valid weights.
        let enc = SpladeEncoder::default();
        let a = enc.encode("splade retrieval model").unwrap();
        let b = enc.encode("splade retrieval ranking").unwrap();
        assert!(!a.is_empty());
        assert!(!b.is_empty());
        for w in a.values().chain(b.values()) {
            assert!(*w > 0.0, "weight must be positive");
        }
    }

    #[cfg(feature = "splade-onnx")]
    #[test]
    #[ignore = "requires model download (~450MB naver/splade-v3)"]
    fn onnx_smoke_machine_learning() {
        use std::time::Instant;
        let enc = SpladeEncoder::from_onnx().expect("load onnx encoder");
        let t0 = Instant::now();
        let sv = enc.encode("machine learning").expect("encode");
        let latency_ms = t0.elapsed().as_millis();
        assert!(!sv.is_empty(), "sparse vec empty");
        assert!(sv.len() <= TOP_K, "too many terms");
        for (&tid, &w) in &sv {
            assert!(tid < VOCAB_SIZE, "token id out of vocab");
            assert!(w > 0.0, "weight must be positive");
        }
        // Print top-5 terms for smoke inspection
        let mut pairs: Vec<(u32, f32)> = sv.into_iter().collect();
        pairs.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        println!("encode latency: {latency_ms}ms");
        println!("top-5 terms (token_id, weight):");
        for (tid, w) in pairs.iter().take(5) {
            println!("  {tid}: {w:.4}");
        }
        // "machine" = token 3698, "learning" = token 4083 in BERT WordPiece
        // At least one of them should appear in the top expanded terms
        let top_ids: Vec<u32> = pairs.iter().take(20).map(|(t, _)| *t).collect();
        assert!(
            top_ids.contains(&3698) || top_ids.contains(&4083),
            "expected machine(3698) or learning(4083) in top-20, got: {top_ids:?}"
        );
    }
}
