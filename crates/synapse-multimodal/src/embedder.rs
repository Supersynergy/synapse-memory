//! `MultimodalEmbedder` trait + `ClipEmbedder` implementation.
//!
//! Model decision:
//! - Feature `multimodal`: real openai/clip-vit-base-patch32 via ONNX/ort (512-d)
//! - Feature `multimodal-dummy`: deterministic placeholder (grayscale histogram + blake3 hash)
//! - Neither: stub that panics (requires a feature)

use std::path::Path;
use anyhow::Result;

/// Shared embed space for text and images.
pub trait MultimodalEmbedder: Send + Sync {
    fn dim(&self) -> usize;
    fn embed_text(&self, text: &str) -> Vec<f32>;
    fn embed_image(&self, path: &Path) -> Result<Vec<f32>>;
}

// ─── Real ONNX CLIP ──────────────────────────────────────────────────────────

#[cfg(feature = "multimodal")]
mod real {
    use super::*;
    use ort::{Environment, Session, SessionBuilder, Value};
    use hf_hub::api::sync::Api;
    use ndarray::{Array2, CowArray};
    use std::sync::Arc;

    const MODEL_REPO: &str = "openai/clip-vit-base-patch32";
    const DIM: usize = 512;

    pub struct ClipEmbedder {
        text_session: Session,
        vision_session: Session,
        _env: Arc<Environment>,
    }

    impl ClipEmbedder {
        pub fn new() -> anyhow::Result<Self> {
            let env = Arc::new(
                Environment::builder().with_name("synapse-clip").build()?
            );
            let api = Api::new()?;
            let repo = api.model(MODEL_REPO.to_string());
            let text_path = repo.get("onnx/text_model.onnx")?;
            let vision_path = repo.get("onnx/vision_model.onnx")?;
            let text_session = SessionBuilder::new(&env)?.with_model_from_file(text_path)?;
            let vision_session = SessionBuilder::new(&env)?.with_model_from_file(vision_path)?;
            Ok(Self { text_session, vision_session, _env: env })
        }
    }

    impl MultimodalEmbedder for ClipEmbedder {
        fn dim(&self) -> usize { DIM }

        fn embed_text(&self, text: &str) -> Vec<f32> {
            // TODO: proper tokenizer (hf-tokenizers)
            // Stub: returns zero vec until tokenizer wired
            tracing::warn!("CLIP text embed stub — tokenizer not yet wired");
            vec![0.0f32; DIM]
        }

        fn embed_image(&self, path: &Path) -> anyhow::Result<Vec<f32>> {
            use image::imageops::FilterType;
            let img = image::open(path)?.resize_exact(224, 224, FilterType::Lanczos3);
            let rgb = img.to_rgb8();
            // Normalize to [-1, 1] with CLIP mean/std
            let mean = [0.48145466f32, 0.4578275, 0.40821073];
            let std  = [0.26862954f32, 0.26130258, 0.27577711];
            let mut pixels = vec![0f32; 3 * 224 * 224];
            for (i, pixel) in rgb.pixels().enumerate() {
                for c in 0..3 {
                    pixels[c * 224 * 224 + i] = (pixel[c] as f32 / 255.0 - mean[c]) / std[c];
                }
            }
            let arr = CowArray::from(
                Array2::from_shape_vec((1, 3 * 224 * 224), pixels)?
            ).into_dyn();
            let inputs = vec![Value::from_array(self.vision_session.allocator(), &arr)?];
            let outputs = self.vision_session.run(inputs)?;
            let embed: &[f32] = outputs[0].try_extract()?.view().as_slice().unwrap().to_vec().as_slice();
            let v: Vec<f32> = outputs[0].try_extract()?.view().as_slice().unwrap().to_vec();
            Ok(l2_norm(v))
        }
    }
}

// ─── Dummy placeholder embedder ──────────────────────────────────────────────

#[cfg(feature = "multimodal-dummy")]
mod dummy {
    use super::*;
    use crate::CLIP_DIM;

    pub struct ClipEmbedder {
        dim: usize,
    }

    impl ClipEmbedder {
        pub fn new() -> Self {
            Self { dim: CLIP_DIM }
        }
    }

    impl MultimodalEmbedder for ClipEmbedder {
        fn dim(&self) -> usize { self.dim }

        fn embed_text(&self, text: &str) -> Vec<f32> {
            text_hash_embed(text, self.dim)
        }

        fn embed_image(&self, path: &Path) -> anyhow::Result<Vec<f32>> {
            let img = image::open(path)?.to_luma8();
            let mut v = image_histogram_embed(&img, self.dim);
            // Mix in path-hash so different images differ
            let path_embed = text_hash_embed(&path.to_string_lossy(), self.dim);
            for (a, b) in v.iter_mut().zip(path_embed.iter()) {
                *a = (*a + *b) / 2.0;
            }
            Ok(l2_norm(v))
        }
    }

    /// Deterministic hash-based text embedding.
    /// Seeds each dimension with BLAKE3 keyed on (text, dim_index).
    fn text_hash_embed(text: &str, dim: usize) -> Vec<f32> {
        let base = blake3::hash(text.as_bytes());
        let mut v = Vec::with_capacity(dim);
        for i in 0..dim {
            let mut h = blake3::Hasher::new_keyed(base.as_bytes());
            h.update(&(i as u32).to_le_bytes());
            let out = h.finalize();
            // Map first 4 bytes to [-1, 1]
            let bits = u32::from_le_bytes(out.as_bytes()[..4].try_into().unwrap());
            v.push((bits as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        l2_norm(v)
    }

    /// 16-bin grayscale histogram padded/repeated to `dim`.
    fn image_histogram_embed(img: &image::GrayImage, dim: usize) -> Vec<f32> {
        let mut bins = [0u32; 16];
        for p in img.pixels() {
            bins[(p[0] >> 4) as usize] += 1;
        }
        let total = img.pixels().count().max(1) as f32;
        let hist: Vec<f32> = bins.iter().map(|&b| b as f32 / total).collect();
        let mut v = Vec::with_capacity(dim);
        for i in 0..dim {
            v.push(hist[i % 16]);
        }
        v
    }
}

// ─── Stub (no feature) ───────────────────────────────────────────────────────

#[cfg(not(any(feature = "multimodal", feature = "multimodal-dummy")))]
mod stub {
    use super::*;
    use crate::CLIP_DIM;

    pub struct ClipEmbedder;
    impl ClipEmbedder {
        pub fn new() -> Self { Self }
    }
    impl MultimodalEmbedder for ClipEmbedder {
        fn dim(&self) -> usize { CLIP_DIM }
        fn embed_text(&self, _: &str) -> Vec<f32> {
            panic!("synapse-multimodal: enable feature `multimodal` or `multimodal-dummy`")
        }
        fn embed_image(&self, _: &Path) -> anyhow::Result<Vec<f32>> {
            panic!("synapse-multimodal: enable feature `multimodal` or `multimodal-dummy`")
        }
    }
}

// ─── Public re-export ────────────────────────────────────────────────────────

#[cfg(feature = "multimodal")]
pub use real::ClipEmbedder;

#[cfg(all(feature = "multimodal-dummy", not(feature = "multimodal")))]
pub use dummy::ClipEmbedder;

#[cfg(not(any(feature = "multimodal", feature = "multimodal-dummy")))]
pub use stub::ClipEmbedder;

// ─── Utility ─────────────────────────────────────────────────────────────────

pub fn l2_norm(mut v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        for x in &mut v { *x /= norm; }
    }
    v
}

pub fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}
