//! MIME detection via magic bytes (`infer` crate).

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MimeKind {
    Jpeg,
    Png,
    Webp,
    Gif,
    Unknown,
}

impl MimeKind {
    pub fn from_path(path: &Path) -> Self {
        if let Ok(kind) = infer::get_from_path(path) {
            match kind.map(|k| k.mime_type()) {
                Some("image/jpeg") => MimeKind::Jpeg,
                Some("image/png") => MimeKind::Png,
                Some("image/webp") => MimeKind::Webp,
                Some("image/gif") => MimeKind::Gif,
                _ => MimeKind::Unknown,
            }
        } else {
            MimeKind::Unknown
        }
    }

    pub fn is_image(self) -> bool {
        !matches!(self, MimeKind::Unknown)
    }
}
