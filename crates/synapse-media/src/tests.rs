#[cfg(test)]
mod tests {
    use crate::db::MediaDb;
    use crate::ingest::{add_image, add_audio};
    use crate::types::MediaKind;
    use std::io::Write;

    fn make_test_image(path: &str) {
        // minimal 1×1 white PNG (89 bytes)
        let png: &[u8] = &[
            0x89,0x50,0x4e,0x47,0x0d,0x0a,0x1a,0x0a,
            0x00,0x00,0x00,0x0d,0x49,0x48,0x44,0x52,
            0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x01,
            0x08,0x02,0x00,0x00,0x00,0x90,0x77,0x53,
            0xde,0x00,0x00,0x00,0x0c,0x49,0x44,0x41,
            0x54,0x08,0xd7,0x63,0xf8,0xcf,0xc0,0x00,
            0x00,0x00,0x02,0x00,0x01,0xe2,0x21,0xbc,
            0x33,0x00,0x00,0x00,0x00,0x49,0x45,0x4e,
            0x44,0xae,0x42,0x60,0x82,
        ];
        std::fs::write(path, png).unwrap();
    }

    #[test]
    fn test_add_and_search_images() {
        let tmp = tempfile::tempdir().unwrap();
        let db = MediaDb::open_in_memory().unwrap();

        for (i, cap) in [("cat on a mat", "cat"), ("dog running", "dog"), ("people in park", "people")].iter().enumerate() {
            let img_path = tmp.path().join(format!("img{i}.png"));
            make_test_image(img_path.to_str().unwrap());
            add_image(&db, img_path.to_str().unwrap(), Some(cap.1)).unwrap();
        }

        let results = db.search("cat", None).unwrap();
        assert!(!results.is_empty(), "search 'cat' should find ≥1 result");
        assert!(results.iter().all(|r| r.kind == MediaKind::Image));

        let people = db.search("people", None).unwrap();
        assert!(!people.is_empty());
    }

    #[test]
    fn test_media_kind_roundtrip() {
        use std::str::FromStr;
        for k in [MediaKind::Image, MediaKind::Video, MediaKind::Audio, MediaKind::Frame, MediaKind::Caption] {
            let s = k.to_string();
            let k2 = MediaKind::from_str(&s).unwrap();
            assert_eq!(format!("{k:?}"), format!("{k2:?}"));
        }
    }

    #[test]
    fn test_ffmpeg_extract_frames() {
        // Skip if ffmpeg not installed
        if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("SKIP: ffmpeg not found");
            return;
        }
        // Create a 2s black video via ffmpeg
        let tmp = tempfile::tempdir().unwrap();
        let video_path = tmp.path().join("test.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y", "-f", "lavfi", "-i", "color=black:size=64x64:rate=25:duration=2",
                "-c:v", "libx264", "-t", "2",
                video_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        if !status.status.success() {
            eprintln!("SKIP: cannot create test video");
            return;
        }

        let db = MediaDb::open_in_memory().unwrap();
        let ids = crate::ingest::add_video(&db, video_path.to_str().unwrap(), 1.0).unwrap();
        // parent + at least 1 frame
        assert!(ids.len() >= 2, "expected ≥2 ids (parent + frames), got {}", ids.len());

        // frames_of query
        let frames = db.frames_of(ids[0]).unwrap();
        assert!(!frames.is_empty());
        assert!(frames.iter().all(|f| f.kind == MediaKind::Frame));
    }

    #[test]
    fn test_comfyui_skips_when_unavailable() {
        use crate::integrations::comfyui::ComfyUi;
        let cui = ComfyUi::new(None);
        if !cui.health() {
            // expected in CI — just confirm health() doesn't panic
            return;
        }
        // If ComfyUI running: submit minimal prompt
        let workflow = serde_json::json!({});
        let _ = cui.submit_workflow(workflow); // may error, that's ok
    }

    #[test]
    fn test_remotion_skips_when_unavailable() {
        use crate::integrations::remotion::RemotionRenderer;
        if !RemotionRenderer::available() {
            eprintln!("SKIP: remotion not found");
            return;
        }
        // If remotion available, just confirm renderer constructs without panic
        let _r = RemotionRenderer::new("/tmp/nonexistent-remotion-project");
    }
}
