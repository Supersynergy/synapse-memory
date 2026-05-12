//! Jina CLIP v2 smoke test — downloads model on first run (~1.7 GB).
//! Run: cargo test -p synapse-multimodal --features clip-jina -- --nocapture
//!
//! Validates:
//!   1. embed_text("cat") → 1024-d unit vector
//!   2. embed_image(cat.png) → 1024-d unit vector
//!   3. cosine(text, image) ≥ 0.2 (cross-modal alignment)

#[cfg(feature = "clip-jina")]
mod jina_smoke {
    use synapse_multimodal::{JinaClipEmbedder, MultimodalEmbedder, embedder::cosine_sim};
    use tempfile::TempDir;
    use image::{RgbImage, Rgb};

    fn make_cat_image(dir: &std::path::Path) -> std::path::PathBuf {
        // Simple orange-ish image as "cat proxy"
        let path = dir.join("cat.png");
        let mut img = RgbImage::new(224, 224);
        for pixel in img.pixels_mut() {
            *pixel = Rgb([200u8, 140, 80]);
        }
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn jina_text_embed_shape_and_norm() {
        let emb = JinaClipEmbedder::new().expect("JinaClipEmbedder::new");
        assert_eq!(emb.dim(), 1024);
        let v = emb.embed_text("cat");
        assert_eq!(v.len(), 1024);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "text embed not unit-norm: {norm}");
    }

    #[test]
    fn jina_image_embed_shape_and_norm() {
        let tmp = TempDir::new().unwrap();
        let cat = make_cat_image(tmp.path());
        let emb = JinaClipEmbedder::new().expect("JinaClipEmbedder::new");
        let v = emb.embed_image(&cat).expect("embed_image");
        assert_eq!(v.len(), 1024);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "image embed not unit-norm: {norm}");
    }

    #[test]
    fn jina_cross_modal_cosine_sane() {
        let tmp = TempDir::new().unwrap();
        let cat = make_cat_image(tmp.path());
        let emb = JinaClipEmbedder::new().expect("JinaClipEmbedder::new");
        let text_vec = emb.embed_text("a cat");
        let img_vec = emb.embed_image(&cat).expect("embed_image");
        let sim = cosine_sim(&text_vec, &img_vec);
        println!("cross-modal cosine(\"a cat\", cat.png) = {sim:.4}");
        assert!(sim >= 0.2, "cross-modal cosine too low: {sim:.4} (expected ≥ 0.2)");
    }
}
