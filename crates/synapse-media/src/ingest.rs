//! High-level ingest API: add_image, add_video, add_audio.
//! video frame-extract: uses `ffmpeg` CLI subprocess (always available).
//! audio transcribe: uses `tawnser` CLI subprocess (whisper.cpp wrapper).

use crate::db::{MediaDb, NewAsset};
use crate::types::{DocId, MediaKind};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

// ─── Image ────────────────────────────────────────────────────────────────────

pub fn add_image(db: &MediaDb, path: &str, caption: Option<&str>) -> Result<DocId> {
    let p = Path::new(path);
    let bytes = std::fs::read(p).context("read image")?;
    let mime = infer::get(&bytes)
        .map(|t| t.mime_type().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let mut meta = HashMap::new();
    if let Ok(img) = image::open(p) {
        let (w, h) = (img.width(), img.height());
        meta.insert("width".into(), serde_json::json!(w));
        meta.insert("height".into(), serde_json::json!(h));
        // thumbnail: 128×128 thumbnail hash (blake3 of raw thumbnail bytes)
        let thumb = img.thumbnail(128, 128);
        let raw = thumb.to_rgb8().into_raw();
        let hash = blake3::hash(&raw).to_hex().to_string();
        meta.insert("thumb_hash".into(), serde_json::json!(hash));
    }

    let id = db.insert(&NewAsset {
        path: path.to_string(),
        kind: MediaKind::Image,
        mime,
        timestamp: None,
        parent_id: None,
        caption: caption.map(str::to_string),
        metadata: meta,
    })?;
    Ok(id)
}

// ─── Video ────────────────────────────────────────────────────────────────────

/// Extract frames at `sample_fps` using ffmpeg CLI, index each frame.
/// Returns DocIds for all frame assets (parent video DocId is first).
pub fn add_video(db: &MediaDb, path: &str, sample_fps: f32) -> Result<Vec<DocId>> {
    let p = Path::new(path);
    let bytes = std::fs::read(p).context("read video header")?;
    let mime = infer::get(&bytes)
        .map(|t| t.mime_type().to_string())
        .unwrap_or_else(|| "video/mp4".to_string());

    let parent_id = db.insert(&NewAsset {
        path: path.to_string(),
        kind: MediaKind::Video,
        mime: mime.clone(),
        timestamp: None,
        parent_id: None,
        caption: None,
        metadata: HashMap::new(),
    })?;

    // Extract frames to temp dir
    let tmp = tempdir()?;
    let pattern = tmp.join("frame_%05d.jpg");
    let fps_str = format!("{sample_fps}");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            path,
            "-vf",
            &format!("fps={fps_str}"),
            "-q:v",
            "5",
            pattern.to_str().unwrap(),
        ])
        .output()
        .context("ffmpeg spawn failed — is ffmpeg installed?")?;

    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("ffmpeg error: {err}");
    }

    let mut ids = vec![parent_id];
    let mut entries: Vec<_> = std::fs::read_dir(&tmp)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for (i, entry) in entries.iter().enumerate() {
        let frame_path = entry.path().to_string_lossy().to_string();
        let ts = i as f32 / sample_fps;
        let id = db.insert(&NewAsset {
            path: frame_path,
            kind: MediaKind::Frame,
            mime: "image/jpeg".to_string(),
            timestamp: Some(ts),
            parent_id: Some(parent_id),
            caption: None,
            metadata: HashMap::new(),
        })?;
        ids.push(id);
    }

    Ok(ids)
}

// ─── Audio ────────────────────────────────────────────────────────────────────

pub fn add_audio(db: &MediaDb, path: &str, transcribe: bool) -> Result<DocId> {
    let bytes = std::fs::read(path).context("read audio")?;
    let mime = infer::get(&bytes)
        .map(|t| t.mime_type().to_string())
        .unwrap_or_else(|| "audio/mpeg".to_string());

    let caption = if transcribe {
        transcribe_via_tawnser(path).ok()
    } else {
        None
    };

    let id = db.insert(&NewAsset {
        path: path.to_string(),
        kind: MediaKind::Audio,
        mime,
        timestamp: None,
        parent_id: None,
        caption: caption.clone(),
        metadata: HashMap::new(),
    })?;

    // Store transcript as a Caption child doc
    if let Some(text) = caption {
        db.insert(&NewAsset {
            path: format!("{path}#transcript"),
            kind: MediaKind::Caption,
            mime: "text/plain".to_string(),
            timestamp: None,
            parent_id: Some(id),
            caption: Some(text),
            metadata: HashMap::new(),
        })?;
    }

    Ok(id)
}

fn transcribe_via_tawnser(path: &str) -> Result<String> {
    let out = Command::new("tawnser")
        .args(["--file", path, "--format", "text"])
        .output()
        .context("tawnser spawn")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("tawnser error: {err}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn tempdir() -> Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("synapse-media-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
