//! ColBERT embedder.
//! Default: deterministic dummy (hash-seeded random), no deps.
//! Feature `colbert-jina`: real jina-colbert-v2 via candle BertModel + linear 768→128 projection.

use anyhow::Result;

pub const TOKEN_DIM: usize = 128;

pub struct ColbertEmbedder {
    pub max_doc_tokens: usize,
    pub max_query_tokens: usize,
    #[cfg(feature = "colbert-jina")]
    inner: Box<jina::JinaInner>,
}

impl Default for ColbertEmbedder {
    fn default() -> Self {
        Self {
            max_doc_tokens: 180,
            max_query_tokens: 32,
            #[cfg(feature = "colbert-jina")]
            inner: panic!("use ColbertEmbedder::from_model() with colbert-jina feature"),
        }
    }
}

impl ColbertEmbedder {
    /// Dummy-only constructor (no model loading).
    pub fn new(max_doc_tokens: usize, max_query_tokens: usize) -> Self {
        Self {
            max_doc_tokens,
            max_query_tokens,
            #[cfg(feature = "colbert-jina")]
            inner: Box::new(jina::JinaInner::new_dummy()),
        }
    }

    /// Load jina-colbert-v2 from a local HF snapshot dir.
    /// Requires feature `colbert-jina`.
    #[cfg(feature = "colbert-jina")]
    pub fn from_model(model_dir: &std::path::Path) -> Result<Self> {
        Ok(Self {
            max_doc_tokens: 180,
            max_query_tokens: 32,
            inner: Box::new(jina::JinaInner::load(model_dir)?),
        })
    }

    /// Download jina-colbert-v2 from HF Hub and load.
    /// Requires feature `colbert-jina`.
    #[cfg(feature = "colbert-jina")]
    pub fn from_hub() -> Result<Self> {
        let dir = jina::download_model()?;
        Self::from_model(&dir)
    }

    /// Embed document → N token vectors [seq, 128] L2-normalised.
    pub fn embed_doc(&self, text: &str) -> Result<Vec<Vec<f32>>> {
        #[cfg(feature = "colbert-jina")]
        {
            return self.inner.embed(text, self.max_doc_tokens, jina::EmbedMode::Doc);
        }
        #[cfg(not(feature = "colbert-jina"))]
        {
            let n = text.split_whitespace().count().clamp(1, self.max_doc_tokens);
            Ok(dummy_embed(text, n, 0x44_6F_63u64))
        }
    }

    /// Embed query → M token vectors [seq, 128] L2-normalised.
    pub fn embed_query(&self, text: &str) -> Result<Vec<Vec<f32>>> {
        #[cfg(feature = "colbert-jina")]
        {
            return self.inner.embed(text, self.max_query_tokens, jina::EmbedMode::Query);
        }
        #[cfg(not(feature = "colbert-jina"))]
        {
            let n = text.split_whitespace().count().clamp(1, self.max_query_tokens);
            Ok(dummy_embed(text, n, 0x51_72_79u64))
        }
    }
}

// ── dummy embedder (default build) ──────────────────────────────────────────

fn dummy_embed(seed_text: &str, n_tokens: usize, salt: u64) -> Vec<Vec<f32>> {
    let seed = seed_text.bytes().fold(salt, |acc, b| {
        acc.wrapping_mul(6364136223846793005u64)
            .wrapping_add(b as u64)
            .wrapping_add(1442695040888963407u64)
    });
    (0..n_tokens)
        .map(|t| {
            let mut s =
                seed.wrapping_add((t as u64).wrapping_mul(0xDEAD_BEEF_CAFE_BABEu64));
            let raw: Vec<f32> = (0..TOKEN_DIM)
                .map(|_| {
                    s = s
                        .wrapping_mul(6364136223846793005u64)
                        .wrapping_add(1442695040888963407u64);
                    (s as i64 as f32) / (i64::MAX as f32)
                })
                .collect();
            l2_norm(raw)
        })
        .collect()
}

pub(crate) fn l2_norm(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

// ── jina-colbert-v2 candle backend ──────────────────────────────────────────

#[cfg(feature = "colbert-jina")]
mod jina {
    use super::{l2_norm, TOKEN_DIM};
    use anyhow::{bail, Context, Result};
    use candle_core::{DType, Device, Tensor};
    use candle_nn::{linear, Linear, Module, VarBuilder};
    use candle_transformers::models::bert::{BertModel, Config as BertConfig};
    use hf_hub::{api::sync::Api, Repo, RepoType};
    use std::path::PathBuf;
    use tokenizers::Tokenizer;

    pub const HF_MODEL_ID: &str = "jinaai/jina-colbert-v2";
    pub const HIDDEN: usize = 768;

    pub enum EmbedMode {
        Doc,
        Query,
    }

    /// Loaded model state.
    pub struct JinaInner {
        bert: BertModel,
        proj: Linear,
        tokenizer: Tokenizer,
        device: Device,
    }

    impl JinaInner {
        /// For the non-jina `new()` path — never actually called because
        /// `ColbertEmbedder::new` with colbert-jina feature uses this.
        #[allow(dead_code)]
        pub fn new_dummy() -> Self {
            panic!("JinaInner::new_dummy — call from_model() instead")
        }

        pub fn load(dir: &std::path::Path) -> Result<Self> {
            // Device: Metal on Apple Silicon, CPU fallback
            let device = if cfg!(target_os = "macos") {
                Device::new_metal(0).unwrap_or(Device::Cpu)
            } else {
                Device::Cpu
            };

            // Config
            let cfg_path = dir.join("config.json");
            let cfg_str = std::fs::read_to_string(&cfg_path)
                .with_context(|| format!("read config.json at {}", cfg_path.display()))?;
            let bert_cfg: BertConfig = serde_json::from_str(&cfg_str)
                .context("parse config.json as BertConfig")?;

            // Weights — try safetensors first, fallback pytorch_model.bin
            let weights_path = {
                let st = dir.join("model.safetensors");
                if st.exists() {
                    st
                } else {
                    dir.join("pytorch_model.bin")
                }
            };
            if !weights_path.exists() {
                bail!("no model weights found in {}", dir.display());
            }

            let vb = unsafe {
                VarBuilder::from_mmaped_safetensors(
                    &[weights_path],
                    DType::F32,
                    &device,
                )?
            };

            let bert = BertModel::load(vb.pp("bert"), &bert_cfg)
                .context("load BertModel")?;

            // Linear projection 768 → 128 (jina-colbert-v2 stores this as
            // `linear_projection.weight` / `.bias`)
            let proj_vb = vb.pp("linear_projection");
            let proj = linear(HIDDEN, TOKEN_DIM, proj_vb)
                .context("load linear_projection (768→128)")?;

            // Tokenizer
            let tok_path = dir.join("tokenizer.json");
            let tokenizer = Tokenizer::from_file(&tok_path)
                .map_err(|e| anyhow::anyhow!("load tokenizer: {e}"))?;

            tracing::info!(
                device = ?device,
                weights = %weights_path.display(),
                "jina-colbert-v2 loaded"
            );

            Ok(Self { bert, proj, tokenizer, device })
        }

        /// Tokenize + forward + project + L2-norm → Vec<Vec<f32>>
        pub fn embed(&self, text: &str, max_tokens: usize, _mode: EmbedMode) -> Result<Vec<Vec<f32>>> {
            let enc = self
                .tokenizer
                .encode(text, true)
                .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;

            let ids: Vec<u32> = enc.get_ids().iter().copied().take(max_tokens).collect();
            let type_ids: Vec<u32> = enc.get_type_ids().iter().copied().take(max_tokens).collect();
            let mask: Vec<u32> = enc.get_attention_mask().iter().copied().take(max_tokens).collect();
            let seq_len = ids.len();

            let input_ids = Tensor::new(ids.as_slice(), &self.device)?.unsqueeze(0)?;
            let token_type_ids = Tensor::new(type_ids.as_slice(), &self.device)?.unsqueeze(0)?;
            let attention_mask = Tensor::new(mask.as_slice(), &self.device)?.unsqueeze(0)?;

            // BERT forward → [1, seq, 768]
            let hidden = self
                .bert
                .forward(&input_ids, &token_type_ids, Some(&attention_mask))?;

            // Remove batch dim → [seq, 768]
            let hidden = hidden.squeeze(0)?;

            // Project → [seq, 128]
            let projected = self.proj.forward(&hidden)?;

            // To CPU f32 Vec
            let data: Vec<f32> = projected.to_dtype(DType::F32)?.to_vec2::<f32>()?.into_iter().flatten().collect();

            // Split into token vecs and L2-norm each
            let vecs: Vec<Vec<f32>> = data
                .chunks(TOKEN_DIM)
                .take(seq_len)
                .map(|chunk| l2_norm(chunk.to_vec()))
                .collect();

            Ok(vecs)
        }
    }

    /// Download model from HF Hub → returns local snapshot dir.
    pub fn download_model() -> Result<PathBuf> {
        let api = Api::new()?;
        let repo = api.repo(Repo::new(HF_MODEL_ID.to_string(), RepoType::Model));
        // Pull the files we need
        let config   = repo.get("config.json")?;
        let _tok     = repo.get("tokenizer.json")?;
        let _weights = repo.get("model.safetensors").or_else(|_| repo.get("pytorch_model.bin"))?;
        // All files land in the same cache dir — return parent of config
        Ok(config.parent().unwrap().to_path_buf())
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        let emb = ColbertEmbedder::new(180, 32);
        let vecs = emb.embed_doc("hello world foo bar").unwrap();
        assert_eq!(vecs.len(), 4);
        assert_eq!(vecs[0].len(), TOKEN_DIM);
        let norm: f32 = vecs[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "not normalised: {norm}");
    }

    /// Smoke test: real jina-colbert-v2 model.
    /// Run: cargo test -p synapse-colbert --features colbert-jina -- --nocapture jina_smoke
    #[cfg(feature = "colbert-jina")]
    #[test]
    fn jina_smoke() {
        use crate::kernel::max_sim;

        let emb = ColbertEmbedder::from_hub().expect("model download/load");
        let text = "ColBERT late interaction";
        let doc_vecs  = emb.embed_doc(text).expect("embed_doc");
        let qry_vecs  = emb.embed_query(text).expect("embed_query");

        let n = doc_vecs.len();
        println!("doc tokens={n}  dim={}", doc_vecs[0].len());
        assert!(n >= 5 && n <= 15, "expected 5-15 tokens, got {n}");
        assert_eq!(doc_vecs[0].len(), TOKEN_DIM);

        // L2-norm check
        for (i, v) in doc_vecs.iter().enumerate() {
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-4, "token {i} not L2-normed: {norm}");
        }

        // Self-match: max_sim(doc, doc) ≈ n_tokens (each qi → exact same dj)
        let self_score = max_sim(&qry_vecs, &doc_vecs);
        let threshold  = 0.95 * (n as f32);
        println!("max_sim self={self_score:.4}  threshold={threshold:.4}");
        assert!(self_score >= threshold, "self max_sim {self_score} < {threshold}");
    }
}
