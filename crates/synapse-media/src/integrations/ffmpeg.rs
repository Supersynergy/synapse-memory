//! Thin ffmpeg CLI wrappers for common video ops.

use anyhow::{Context, Result};
use std::process::Command;

pub struct FfmpegOpts<'a> {
    pub input: &'a str,
    pub output: &'a str,
}

/// Extract frames at given fps into output_dir (pattern: frame_%05d.jpg).
pub fn extract_frames(input: &str, output_pattern: &str, fps: f32) -> Result<()> {
    let status = Command::new("ffmpeg")
        .args([
            "-y", "-i", input,
            "-vf", &format!("fps={fps}"),
            "-q:v", "5",
            output_pattern,
        ])
        .output()
        .context("ffmpeg not found")?;
    if !status.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&status.stderr));
    }
    Ok(())
}

/// Transcode input → output (format inferred from extension).
pub fn transcode(opts: FfmpegOpts<'_>, extra_args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-y", "-i", opts.input]);
    cmd.args(extra_args);
    cmd.arg(opts.output);
    let out = cmd.output().context("ffmpeg spawn")?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// Concat list of files via concat demuxer.
pub fn concat(inputs: &[&str], output: &str) -> Result<()> {
    let tmp = std::env::temp_dir().join(format!("concat_{}.txt", std::process::id()));
    let content: String = inputs.iter().map(|p| format!("file '{p}'\n")).collect();
    std::fs::write(&tmp, content)?;
    let out = Command::new("ffmpeg")
        .args([
            "-y", "-f", "concat", "-safe", "0",
            "-i", tmp.to_str().unwrap(),
            "-c", "copy", output,
        ])
        .output()
        .context("ffmpeg spawn")?;
    let _ = std::fs::remove_file(&tmp);
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// Trim [start_sec, end_sec].
pub fn trim(input: &str, output: &str, start: f32, end: f32) -> Result<()> {
    let out = Command::new("ffmpeg")
        .args([
            "-y", "-i", input,
            "-ss", &start.to_string(),
            "-to", &end.to_string(),
            "-c", "copy", output,
        ])
        .output()
        .context("ffmpeg spawn")?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}
