//! In-memory cross-modal index for smoke tests and small collections.
//! For production: wire into `synapse-core` `Db::add_with_embed`.

use crate::embedder::{MultimodalEmbedder, cosine_sim};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Text,
    Image,
}

#[derive(Debug, Clone)]
pub struct ModalDoc {
    pub id: String,
    pub content: String,
    pub kind: ModalKind,
    pub embed: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct ModalHit {
    pub id: String,
    pub content: String,
    pub kind: ModalKind,
    pub score: f32,
}

pub struct CrossModalIndex {
    dim: usize,
    docs: Vec<ModalDoc>,
}

impl CrossModalIndex {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            docs: Vec::new(),
        }
    }

    pub fn add_text(&mut self, id: &str, text: &str, emb: &dyn MultimodalEmbedder) {
        let embed = emb.embed_text(text);
        self.docs.push(ModalDoc {
            id: id.to_string(),
            content: text.to_string(),
            kind: ModalKind::Text,
            embed,
        });
    }

    pub fn add_image(
        &mut self,
        id: &str,
        path: &Path,
        caption: Option<&str>,
        emb: &dyn MultimodalEmbedder,
    ) -> anyhow::Result<()> {
        let embed = emb.embed_image(path)?;
        let content = caption
            .map(|c| c.to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        self.docs.push(ModalDoc {
            id: id.to_string(),
            content,
            kind: ModalKind::Image,
            embed,
        });
        Ok(())
    }

    /// Text query → ranked hits across both text and image docs.
    pub fn query_text(
        &self,
        query: &str,
        emb: &dyn MultimodalEmbedder,
        top_k: usize,
    ) -> Vec<ModalHit> {
        let q_embed = emb.embed_text(query);
        self.rank(&q_embed, top_k)
    }

    /// Image query → ranked hits across both text and image docs.
    pub fn query_image(
        &self,
        path: &Path,
        emb: &dyn MultimodalEmbedder,
        top_k: usize,
    ) -> anyhow::Result<Vec<ModalHit>> {
        let q_embed = emb.embed_image(path)?;
        Ok(self.rank(&q_embed, top_k))
    }

    fn rank(&self, q: &[f32], top_k: usize) -> Vec<ModalHit> {
        let mut scored: Vec<(f32, &ModalDoc)> = self
            .docs
            .iter()
            .map(|d| (cosine_sim(q, &d.embed), d))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        scored.truncate(top_k);
        scored
            .into_iter()
            .map(|(score, d)| ModalHit {
                id: d.id.clone(),
                content: d.content.clone(),
                kind: d.kind,
                score,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
    pub fn dim(&self) -> usize {
        self.dim
    }
}
