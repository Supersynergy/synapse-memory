use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub type DocId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Frame,
    Caption,
}

impl std::fmt::Display for MediaKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaKind::Image => write!(f, "image"),
            MediaKind::Video => write!(f, "video"),
            MediaKind::Audio => write!(f, "audio"),
            MediaKind::Frame => write!(f, "frame"),
            MediaKind::Caption => write!(f, "caption"),
        }
    }
}

impl std::str::FromStr for MediaKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "image" => Ok(MediaKind::Image),
            "video" => Ok(MediaKind::Video),
            "audio" => Ok(MediaKind::Audio),
            "frame" => Ok(MediaKind::Frame),
            "caption" => Ok(MediaKind::Caption),
            other => anyhow::bail!("unknown MediaKind: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: DocId,
    pub path: String,
    pub kind: MediaKind,
    pub mime: String,
    /// Seconds offset within parent video (frames only).
    pub timestamp: Option<f32>,
    /// Parent video/audio DocId (frames/captions only).
    pub parent_asset: Option<DocId>,
    pub metadata: HashMap<String, Value>,
}
