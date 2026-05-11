//! synapse-media: multimodal asset-DB + retrieval integration layer.
//!
//! External tools (ComfyUI, Remotion, ffmpeg) consume indexed assets via this crate.
//! Feature gates:
//!   - `video`     — ffmpeg-next native bindings (requires libavcodec)
//!   - `clip-local` — candle CLIP local inference (heavy)
//!
//! Default build uses subprocess CLI for ffmpeg + tawnser (always-available path).

pub mod db;
pub mod ingest;
pub mod integrations;
pub mod types;
pub mod video_embed;

pub use db::MediaDb;
pub use ingest::{add_audio, add_image, add_video};
pub use types::{DocId, MediaAsset, MediaKind};

#[cfg(test)]
mod tests;
