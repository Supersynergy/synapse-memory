//! `MultimodalEmbedder` trait + `ClipEmbedder` implementations.
//!
//! Model decision:
//! - Feature `clip-jina`: jinaai/jina-clip-v2 via ONNX/ort + HF tokenizer (1024-d, 89 langs)
//! - Feature `multimodal`: openai/clip-vit-base-patch32 via ONNX/ort (512-d)
//! - Feature `multimodal-dummy`: deterministic placeholder (grayscale histogram + blake3 hash)
//! - Neither: stub that panics

use std::path::Path;
use anyhow::Result;

/// Shared embed space for text and images.
pub trait MultimodalEmbedder: Send + Sync {
    fn dim(&self) -> usize;
    fn embed_text(&self, text: &str) -> Vec<f32>;
    fn embed_image(&self, path: &Path) -> Result<Vec<f32>>;
}

// ─── jina-clip-v2 (clip-jina feature) ────────────────────────────────────────

#[cfg(feature = "clip-jina")]
pub mod jina {
    use super::*;
    use std::sync::Mutex;
    use ort::{session::Session, value::Tensor};
    use hf_hub::api::sync::Api;
    use tokenizers::Tokenizer;

    const MODEL_REPO: &str = "jinaai/jina-clip-v2";
    const TEXT_ONNX: &str = "onnx/text_model.onnx";
    const VISION_ONNX: &str = "onnx/vision_model.onnx";
    pub const DIM: usize = 1024;
    const IMG_SIZE: u32 = 224;
    const MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
    const STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];

    pub struct JinaClipEmbedder {
        text_session: Mutex<Session>,
        vision_session: Mutex<Session>,
        tokenizer: Tokenizer,
    }

    impl JinaClipEmbedder {
        pub fn new() -> anyhow::Result<Self> {
            let api = Api::new()?;
            let repo = api.model(MODEL_REPO.to_string());
            let text_path = repo.get(TEXT_ONNX)?;
            let vision_path = repo.get(VISION_ONNX)?;
            let tok_path = repo.get("tokenizer.json")?;
            let tokenizer = Tokenizer::from_file(&tok_path)
                .map_err(|e| anyhow::anyhow!("tokenizer load: {e}"))?;
            let text_session = Session::builder()?.commit_from_file(&text_path)?;
            let vision_session = Session::builder()?.commit_from_file(&vision_path)?;
            Ok(Self {
                text_session: Mutex::new(text_session),
                vision_session: Mutex::new(vision_session),
                tokenizer,
            })
        }
    }

    impl MultimodalEmbedder for JinaClipEmbedder {
        fn dim(&self) -> usize { DIM }

        fn embed_text(&self, text: &str) -> Vec<f32> {
            match self.embed_text_inner(text) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("jina embed_text error: {e}");
                    vec![0.0f32; DIM]
                }
            }
        }

        fn embed_image(&self, path: &Path) -> anyhow::Result<Vec<f32>> {
            self.embed_image_inner(path)
        }
    }

    impl JinaClipEmbedder {
        fn embed_text_inner(&self, text: &str) -> anyhow::Result<Vec<f32>> {
            let encoding = self.tokenizer.encode(text, true)
                .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
            let ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
            let mask: Vec<i64> = encoding.get_attention_mask().iter().map(|&x| x as i64).collect();
            let len = ids.len();
            let input_ids = Tensor::<i64>::from_array(([1usize, len], ids))?;
            let attention_mask = Tensor::<i64>::from_array(([1usize, len], mask))?;
            let mut guard = self.text_session
                .lock().map_err(|_| anyhow::anyhow!("text session mutex poisoned"))?;
            let outputs = guard.run(
                ort::inputs!["input_ids" => input_ids, "attention_mask" => attention_mask]
            )?;
            let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
            Ok(super::l2_norm(data.to_vec()))
        }

        fn embed_image_inner(&self, path: &Path) -> anyhow::Result<Vec<f32>> {
            use image::imageops::FilterType;
            let img = image::open(path)?
                .resize_exact(IMG_SIZE, IMG_SIZE, FilterType::Lanczos3)
                .to_rgb8();
            let hw = (IMG_SIZE * IMG_SIZE) as usize;
            let mut pixels = vec![0f32; 3 * hw];
            for (i, pixel) in img.pixels().enumerate() {
                for c in 0..3 {
                    pixels[c * hw + i] = (pixel[c] as f32 / 255.0 - MEAN[c]) / STD[c];
                }
            }
            let tensor = Tensor::<f32>::from_array((
                [1usize, 3, IMG_SIZE as usize, IMG_SIZE as usize],
                pixels,
            ))?;
            let mut guard = self.vision_session
                .lock().map_err(|_| anyhow::anyhow!("vision session mutex poisoned"))?;
            let outputs = guard.run(ort::inputs!["pixel_values" => tensor])?;
            let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
            Ok(super::l2_norm(data.to_vec()))
        }
    }
}

// ─── Real ONNX CLIP (openai/clip-vit-base-patch32, 512-d) ────────────────────

#[cfg(all(feature = "multimodal", not(feature = "clip-jina")))]
mod real {
    use super::*;
    use ort::{session::Session, value::Tensor};
    use hf_hub::api::sync::Api;

    const MODEL_REPO: &str = "openai/clip-vit-base-patch32";
    const DIM: usize = 512;
    const IMG_SIZE: u32 = 224;
    const MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
    const STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];

    pub struct ClipEmbedder {
        text_session: std::sync::Mutex<Session>,
        vision_session: std::sync::Mutex<Session>,
    }

    impl ClipEmbedder {
        pub fn new() -> anyhow::Result<Self> {
            let api = Api::new()?;
            let repo = api.model(MODEL_REPO.to_string());
            let text_path = repo.get("onnx/text_model.onnx")?;
            let vision_path = repo.get("onnx/vision_model.onnx")?;
            let text_session = Session::builder()?.commit_from_file(&text_path)?;
            let vision_session = Session::builder()?.commit_from_file(&vision_path)?;
            Ok(Self {
                text_session: std::sync::Mutex::new(text_session),
                vision_session: std::sync::Mutex::new(vision_session),
            })
        }
    }

    impl MultimodalEmbedder for ClipEmbedder {
        fn dim(&self) -> usize { DIM }

        fn embed_text(&self, _text: &str) -> Vec<f32> {
            tracing::warn!("CLIP openai text stub — tokenizer not wired; use clip-jina feature");
            vec![0.0f32; DIM]
        }

        fn embed_image(&self, path: &Path) -> anyhow::Result<Vec<f32>> {
            use image::imageops::FilterType;
            let img = image::open(path)?
                .resize_exact(IMG_SIZE, IMG_SIZE, FilterType::Lanczos3)
                .to_rgb8();
            let hw = (IMG_SIZE * IMG_SIZE) as usize;
            let mut pixels = vec![0f32; 3 * hw];
            for (i, pixel) in img.pixels().enumerate() {
                for c in 0..3 {
                    pixels[c * hw + i] = (pixel[c] as f32 / 255.0 - MEAN[c]) / STD[c];
                }
            }
            let tensor = Tensor::<f32>::from_array((
                [1usize, 3, IMG_SIZE as usize, IMG_SIZE as usize],
                pixels,
            ))?;
            let mut guard = self.vision_session
                .lock().map_err(|_| anyhow::anyhow!("vision session mutex poisoned"))?;
            let outputs = guard.run(ort::inputs!["pixel_values" => tensor])?;
            let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
            Ok(super::l2_norm(data.to_vec()))
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
            let path_embed = text_hash_embed(&path.to_string_lossy(), self.dim);
            for (a, b) in v.iter_mut().zip(path_embed.iter()) {
                *a = (*a + *b) / 2.0;
            }
            Ok(l2_norm(v))
        }
    }

    fn text_hash_embed(text: &str, dim: usize) -> Vec<f32> {
        let base = blake3::hash(text.as_bytes());
        let mut v = Vec::with_capacity(dim);
        for i in 0..dim {
            let mut h = blake3::Hasher::new_keyed(base.as_bytes());
            h.update(&(i as u32).to_le_bytes());
            let out = h.finalize();
            let bits = u32::from_le_bytes(out.as_bytes()[..4].try_into().unwrap());
            v.push((bits as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        l2_norm(v)
    }

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

#[cfg(not(any(feature = "multimodal", feature = "multimodal-dummy", feature = "clip-jina")))]
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
            panic!("synapse-multimodal: enable feature `clip-jina`, `multimodal`, or `multimodal-dummy`")
        }
        fn embed_image(&self, _: &Path) -> anyhow::Result<Vec<f32>> {
            panic!("synapse-multimodal: enable feature `clip-jina`, `multimodal`, or `multimodal-dummy`")
        }
    }
}

// ─── Public re-exports ───────────────────────────────────────────────────────

// clip-jina: expose JinaClipEmbedder as ClipEmbedder (1024-d, default real)
#[cfg(feature = "clip-jina")]
pub use jina::JinaClipEmbedder as ClipEmbedder;

#[cfg(all(feature = "multimodal", not(feature = "clip-jina")))]
pub use real::ClipEmbedder;

#[cfg(all(feature = "multimodal-dummy", not(feature = "multimodal"), not(feature = "clip-jina")))]
pub use dummy::ClipEmbedder;

#[cfg(not(any(feature = "multimodal", feature = "multimodal-dummy", feature = "clip-jina")))]
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
