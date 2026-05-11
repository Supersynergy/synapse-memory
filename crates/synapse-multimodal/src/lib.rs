//! Multimodal CLIP-style shared embedding space.
//!
//! # Design
//!
//! - [`MultimodalEmbedder`] trait: `embed_text` + `embed_image` → shared 512-d f32 space
//! - [`ClipEmbedder`]: Real ONNX CLIP (feature `multimodal`) or dummy (feature `multimodal-dummy`)
//! - [`MimeKind`]: JPEG/PNG/WEBP/GIF/PDF/Unknown via `infer` magic-bytes
//! - [`CrossModalIndex`]: in-memory index over `(doc_id, embed, kind)` for demo/smoke
//!
//! # Cross-modal query
//!
//! ```no_run
//! # #[cfg(feature = "multimodal-dummy")]
//! # {
//! use synapse_multimodal::{ClipEmbedder, CrossModalIndex, MultimodalEmbedder};
//! use std::path::Path;
//! let emb = ClipEmbedder::new();
//! let mut idx = CrossModalIndex::new(emb.dim());
//! idx.add_image("cat.jpg", Path::new("cat.jpg"), None, &emb).unwrap();
//! idx.add_text("caption_dog", "a dog running", &emb);
//! let hits = idx.query_text("cat", &emb, 3);
//! # }
//! ```

pub mod embedder;
pub mod index;
pub mod mime;
pub mod storage;

pub use embedder::{ClipEmbedder, MultimodalEmbedder};
pub use index::{CrossModalIndex, ModalHit, ModalKind};
pub use mime::MimeKind;

/// CLIP shared embed dimension — 512 for ViT-B/32.
pub const CLIP_DIM: usize = 512;
