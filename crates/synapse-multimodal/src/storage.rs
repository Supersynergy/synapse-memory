//! Bridge to synapse-core `Db::add_with_embed`.
//!
//! Thin wrapper — keeps synapse-multimodal independent from synapse-core at
//! crate level (no circular dep). Callers link both and call `add_image_to_db`.

use crate::embedder::MultimodalEmbedder;
use crate::mime::MimeKind;
use std::path::Path;

/// Metadata stored alongside the image doc.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ImageMeta {
    pub path: String,
    pub mime: String,
    pub caption: Option<String>,
    pub embed_dim: usize,
}

pub type PreparedImageDoc = (String, Vec<f32>, String);

/// Serialize an image into `(doc_content, embed, meta_json)` for Db insertion.
pub fn prepare_image_doc(
    path: &Path,
    caption: Option<&str>,
    emb: &dyn MultimodalEmbedder,
) -> anyhow::Result<PreparedImageDoc> {
    let mime = MimeKind::from_path(path);
    let embed = emb.embed_image(path)?;
    let content = caption
        .map(|c| c.to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let meta = serde_json::to_string(&ImageMeta {
        path: path.to_string_lossy().to_string(),
        mime: format!("{:?}", mime),
        caption: caption.map(|c| c.to_string()),
        embed_dim: emb.dim(),
    })?;
    Ok((content, embed, meta))
}
