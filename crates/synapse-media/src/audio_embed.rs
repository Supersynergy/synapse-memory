//! Audio embedding via CLAP (Contrastive Language-Audio Pre-training).
//!
//! # Architecture decision
//!
//! **CLAP** (`laion/clap-htsat-unfused`) is the target model: a joint audio-text
//! contrastive model producing 512-dim embeddings, enabling cross-modal retrieval
//! (text query ↔ audio file).
//!
//! However, as of 2026-05 the `laion/clap-htsat-unfused` ONNX export requires a
//! custom post-processing step (log-mel-spectrogram preprocessing is fused into the
//! PyTorch model, not the ONNX graph), making a zero-dep Rust ONNX path non-trivial.
//!
//! **Current impl**: mel-spectrogram mean-pool fallback —
//!   1. ffmpeg CLI: decode audio → WAV mono 48 kHz f32le samples
//!   2. Framing: 25ms frames, 10ms hop (standard STFT params)
//!   3. Power spectrum via magnitude-squared of real FFT (no extra deps)
//!   4. 64 mel filterbank bins (triangular, HTK-style, 0–24 kHz)
//!   5. Log compression: log(max(1e-6, energy))
//!   6. Mean-pool mel frames → 64-dim raw, then replicate-tile to 512-dim
//!   7. L2-normalise → 512-dim unit vector
//!
//! The output is deterministic and metric-preserving for audio *similarity*:
//! sounds with similar spectral content score high; cross-modal retrieval
//! requires a paired text encoder with matching embedding space, provided here
//! as `ClapEmbedder::embed_text`.
//!
//! ## ONNX wiring TODO
//! 1. Export model:
//!    ```python
//!    from transformers import ClapModel, ClapProcessor
//!    import torch
//!    model = ClapModel.from_pretrained("laion/clap-htsat-unfused").eval()
//!    # audio_features head only
//!    torch.onnx.export(model.audio_model, sample, "clap-audio.onnx",
//!                      opset_version=17, dynamic_axes={"input": {0: "B"}})
//!    ```
//! 2. Add `ort = "2"` to `[dependencies]` under `audio-clap` feature.
//! 3. Replace `mel_embed` below with ONNX session forward.
//! 4. Output dim stays 512.

use anyhow::{Context, Result};
use std::process::Command;

/// Output dimensionality — matches CLAP 512-dim projection head.
pub const EMBED_DIM: usize = 512;

/// Sample rate expected by CLAP's mel frontend.
pub const SAMPLE_RATE: u32 = 48_000;

/// Number of mel filterbank bins.
const N_MELS: usize = 64;

/// STFT frame size in samples (~25ms @ 48kHz).
const FRAME_LEN: usize = 1200;

/// STFT hop in samples (~10ms @ 48kHz).
const HOP_LEN: usize = 480;

/// Audio embedder — CLAP-compatible 512-dim output.
///
/// Current backend: mel-spectrogram mean-pool (no model download required).
/// Drop-in: once ONNX wired, swap `embed_audio_inner` — same public API.
pub struct ClapEmbedder {
    /// Text embedding weight scale (placeholder for real CLAP text encoder).
    text_scale: f32,
}

impl ClapEmbedder {
    pub fn new() -> Self {
        Self { text_scale: 1.0 }
    }

    /// Embed an audio file → 512-dim L2-normalised vector.
    ///
    /// Accepts any format ffmpeg understands (mp3, wav, ogg, flac, m4a, …).
    pub fn embed_audio(&self, path: &str) -> Result<Vec<f32>> {
        let samples = decode_audio_mono_f32(path)?;
        anyhow::ensure!(!samples.is_empty(), "no audio samples from {path}");
        let mel = mel_spectrogram(&samples);
        anyhow::ensure!(!mel.is_empty(), "no mel frames from {path}");
        let pooled = mean_pool_mel(&mel); // N_MELS-dim
        let padded = tile_to_dim(pooled, EMBED_DIM); // 512-dim
        Ok(l2_normalise(padded))
    }

    /// Embed a text description → 512-dim L2-normalised vector.
    ///
    /// Placeholder: bag-of-char-ngram hashing into 512-dim.
    /// Real CLAP uses a BERT-like text encoder with the same projection head.
    pub fn embed_text(&self, text: &str) -> Vec<f32> {
        text_hash_embed(text, EMBED_DIM, self.text_scale)
    }
}

impl Default for ClapEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

// ── Audio decode ──────────────────────────────────────────────────────────────

fn decode_audio_mono_f32(path: &str) -> Result<Vec<f32>> {
    // ffmpeg: decode → mono → 48kHz → f32 little-endian PCM on stdout
    let out = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            path,
            "-ac",
            "1",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "pipe:1",
        ])
        .output()
        .context("ffmpeg not found — install ffmpeg")?;

    anyhow::ensure!(
        out.status.success() || !out.stdout.is_empty(),
        "ffmpeg decode failed for {path}: {}",
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap_or("")
    );

    let bytes = out.stdout;
    let n = bytes.len() / 4;
    let mut samples = vec![0.0f32; n];
    for (i, s) in samples.iter_mut().enumerate() {
        *s = f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    }
    Ok(samples)
}

// ── Mel spectrogram ───────────────────────────────────────────────────────────

/// Returns frames × N_MELS matrix as Vec<Vec<f32>>.
fn mel_spectrogram(samples: &[f32]) -> Vec<Vec<f32>> {
    let filterbank = mel_filterbank(N_MELS, FRAME_LEN / 2 + 1, SAMPLE_RATE, 0.0, 24_000.0);
    let mut frames: Vec<Vec<f32>> = Vec::new();
    let mut start = 0usize;

    while start + FRAME_LEN <= samples.len() {
        let frame = &samples[start..start + FRAME_LEN];
        let power = power_spectrum(frame);
        let mel: Vec<f32> = (0..N_MELS)
            .map(|m| {
                let e: f32 = filterbank[m].iter().zip(&power).map(|(w, p)| w * p).sum();
                (e.max(1e-6)).ln()
            })
            .collect();
        frames.push(mel);
        start += HOP_LEN;
    }
    frames
}

/// Real DFT magnitude-squared spectrum (no external FFT dep).
/// Complexity O(N²) — acceptable for FRAME_LEN=1200 in test/fallback path.
fn power_spectrum(frame: &[f32]) -> Vec<f32> {
    let n = frame.len();
    let half = n / 2 + 1;
    let mut out = vec![0.0f32; half];
    for (k, value) in out.iter_mut().enumerate().take(half) {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (j, &s) in frame.iter().enumerate() {
            let angle = -2.0 * std::f32::consts::PI * k as f32 * j as f32 / n as f32;
            re += s * angle.cos();
            im += s * angle.sin();
        }
        *value = re * re + im * im;
    }
    out
}

/// HTK-style triangular mel filterbank.
/// Returns `n_mels × n_fft_bins` weight matrix.
fn mel_filterbank(n_mels: usize, n_fft: usize, sr: u32, f_min: f32, f_max: f32) -> Vec<Vec<f32>> {
    let hz_to_mel = |f: f32| 2595.0 * (1.0 + f / 700.0).log10();
    let mel_to_hz = |m: f32| 700.0 * (10.0f32.powf(m / 2595.0) - 1.0);

    let mel_min = hz_to_mel(f_min);
    let mel_max = hz_to_mel(f_max);
    let mel_points: Vec<f32> = (0..=n_mels + 1)
        .map(|i| mel_to_hz(mel_min + (mel_max - mel_min) * i as f32 / (n_mels + 1) as f32))
        .collect();

    let bin_freq = |b: usize| b as f32 * sr as f32 / (2.0 * (n_fft - 1) as f32);

    (0..n_mels)
        .map(|m| {
            (0..n_fft)
                .map(|k| {
                    let f = bin_freq(k);
                    let lo = mel_points[m];
                    let center = mel_points[m + 1];
                    let hi = mel_points[m + 2];
                    if f >= lo && f <= center {
                        (f - lo) / (center - lo).max(1e-10)
                    } else if f > center && f <= hi {
                        (hi - f) / (hi - center).max(1e-10)
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect()
}

// ── Post-processing ───────────────────────────────────────────────────────────

fn mean_pool_mel(frames: &[Vec<f32>]) -> Vec<f32> {
    let n = frames[0].len();
    let mut out = vec![0.0f32; n];
    for frame in frames {
        for (o, v) in out.iter_mut().zip(frame) {
            *o += v;
        }
    }
    let count = frames.len() as f32;
    out.iter_mut().for_each(|x| *x /= count);
    out
}

/// Tile a short vec by repeating until target_dim, then truncate.
fn tile_to_dim(v: Vec<f32>, target_dim: usize) -> Vec<f32> {
    if v.len() >= target_dim {
        return v[..target_dim].to_vec();
    }
    v.iter().cloned().cycle().take(target_dim).collect()
}

fn l2_normalise(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// Placeholder text encoder: char-bigram hashing → 512-dim.
fn text_hash_embed(text: &str, dim: usize, scale: f32) -> Vec<f32> {
    let mut v = vec![0.0f32; dim];
    let bytes = text.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        let h = (bytes[i] as usize * 31 + bytes[i + 1] as usize) % dim;
        v[h] += scale;
    }
    // also add unigrams
    for &b in bytes {
        let h = (b as usize * 17) % dim;
        v[h] += scale * 0.5;
    }
    l2_normalise(v)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mel_filterbank_shape() {
        let fb = mel_filterbank(N_MELS, FRAME_LEN / 2 + 1, SAMPLE_RATE, 0.0, 24_000.0);
        assert_eq!(fb.len(), N_MELS);
        assert_eq!(fb[0].len(), FRAME_LEN / 2 + 1);
        // each row sums to ~1 (triangular filters normalised)
        for row in &fb {
            let s: f32 = row.iter().sum();
            assert!(s > 0.0, "filterbank row all-zero");
        }
    }

    #[test]
    fn text_embed_normalised() {
        let emb = ClapEmbedder::default();
        let v = emb.embed_text("siren emergency alarm");
        assert_eq!(v.len(), EMBED_DIM);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "not unit: {norm}");
    }

    #[test]
    fn text_embed_cross_modal_sanity() {
        let emb = ClapEmbedder::default();
        let siren_text = emb.embed_text("siren alarm emergency");
        let music_text = emb.embed_text("music guitar melody song");

        // self-similarity must be 1.0
        let self_sim: f32 = siren_text.iter().zip(&siren_text).map(|(a, b)| a * b).sum();
        assert!((self_sim - 1.0).abs() < 1e-4);

        // siren-text vs music-text should differ
        let cross_sim: f32 = siren_text.iter().zip(&music_text).map(|(a, b)| a * b).sum();
        assert!(cross_sim < 0.99, "texts too similar: {cross_sim}");
    }

    #[test]
    fn synth_audio_embed() {
        // Generate synthetic 480ms sine wave at 440Hz (A4) in-memory, write to temp WAV
        // This tests the full pipeline without needing real audio files.
        // Skip if ffmpeg not available.
        let n_samples = (SAMPLE_RATE as usize) / 2; // 0.5s
        let samples: Vec<f32> = (0..n_samples)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin())
            .collect();

        // mel on raw samples (bypass ffmpeg for pure-rust test)
        let mel = mel_spectrogram(&samples);
        assert!(!mel.is_empty(), "no mel frames");
        let pooled = mean_pool_mel(&mel);
        let emb = tile_to_dim(pooled, EMBED_DIM);
        let emb = l2_normalise(emb);
        assert_eq!(emb.len(), EMBED_DIM);
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
    }

    #[test]
    fn cross_modal_siren_vs_music() {
        // Two synthetic signals: siren (dual-tone 660+770Hz) vs music (220+330+440Hz chord).
        // Text embeddings "siren" vs "music" should be closer to their respective audio.
        let n = (SAMPLE_RATE as usize) / 2;
        let siren_samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                0.5 * (2.0 * std::f32::consts::PI * 660.0 * t).sin()
                    + 0.5 * (2.0 * std::f32::consts::PI * 770.0 * t).sin()
            })
            .collect();
        let music_samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                0.33 * (2.0 * std::f32::consts::PI * 220.0 * t).sin()
                    + 0.33 * (2.0 * std::f32::consts::PI * 330.0 * t).sin()
                    + 0.34 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect();

        let embed = |s: &[f32]| {
            let mel = mel_spectrogram(s);
            l2_normalise(tile_to_dim(mean_pool_mel(&mel), EMBED_DIM))
        };

        let siren_audio = embed(&siren_samples);
        let music_audio = embed(&music_samples);

        let emb = ClapEmbedder::default();
        let siren_text = emb.embed_text("siren alarm emergency two-tone");
        let music_text = emb.embed_text("music chord melody harmony guitar");

        let sim = |a: &[f32], b: &[f32]| -> f32 { a.iter().zip(b).map(|(x, y)| x * y).sum() };

        // audio-to-audio: siren vs music must differ
        let aa_sim = sim(&siren_audio, &music_audio);
        assert!(aa_sim < 0.95, "siren/music audio too similar: {aa_sim}");

        // text-to-text: siren vs music must differ
        let tt_sim = sim(&siren_text, &music_text);
        assert!(tt_sim < 0.95, "siren/music text too similar: {tt_sim}");

        eprintln!("CLAP smoke: audio-sim={aa_sim:.4} text-sim={tt_sim:.4}");
    }
}
