# PHASE-5: MLX Metal Embedder

Date: 2026-04-25 · Owner: Team Zeta ζ1/ζ2/ζ3
Source: subagent a07cda55a16fe6332 · Verdict: ✅ GO

## Goal
Default to MLX Metal embedder on Apple Silicon. Fall back to ONNX CPU elsewhere. **Target: 30ms → 5-8ms (4-6× faster embed).**

## Critical Finding
Architecture is **already trait-based** in `embedder_trait.rs`. Wire-up is 90% done.
Just need MLX implementation module + feature flag.

## Current Embedder API
- `Embedder::new()` / `Embedder::new_with_cache(path)`
- Methods: `embed_one(text)`, `embed_batch(texts)`
- Backend: fastembed BGE-small ONNX CPU, global session pool, redb hash cache

## Trait Design

```rust
// crates/synapse-core/src/embedder_trait.rs (existing, extend)
#[async_trait]
pub trait EmbedderBackend: Send + Sync {
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>>;
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
    fn name(&self) -> &str;
}
```

Two impls:
- `MlxMetalEmbedder` (apple-silicon only) — uses `mlx-rs` crate
- `OnnxEmbedder` (cross-platform) — current fastembed wrap

## Cargo Features

```toml
[features]
default = ["embed-onnx"]
embed-onnx = ["dep:fastembed"]
embed-mlx  = ["dep:mlx-rs"]   # cfg(target_os = "macos", target_arch = "aarch64")
```

## Runtime Routing

```rust
pub fn pick_embedder() -> Box<dyn EmbedderBackend> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "embed-mlx"))]
    {
        if is_apple_silicon() { return Box::new(MlxMetalEmbedder::new()); }
    }
    Box::new(OnnxEmbedder::new())
}

fn is_apple_silicon() -> bool {
    std::process::Command::new("sysctl")
        .args(["-n", "hw.optional.arm.FEAT_FP16"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}
```

## Model Format
- BGE-small weights via HuggingFace Hub: `Xenova/bge-small-en-v1.5`
- MLX wants safetensors → ship pre-converted weights at `huggingface.co/supersynergy/bge-small-mlx`
- One-time conversion: `python -m mlx_lm.convert --hf-path Xenova/bge-small-en-v1.5 --mlx-path bge-small-mlx`

## Crate Pick
`mlx-rs` (pure-Rust binding) verified — alternatives `mlx-fast-rs` less mature. Stable API.

## 4 Implementation Steps (Days 57-65)

| Day | Step |
|---|---|
| 57-58 | Create `crates/synapse-core/src/embed_mlx.rs` (impl `EmbedderBackend`) |
| 59-60 | Wire to `pick_embedder()` runtime detection + Cargo feature |
| 61-62 | Convert BGE-small to MLX safetensors, upload to HF |
| 63-65 | Bench validation + integrate into daemon embedder pool |

## Validation Bench (DoD)
- 100 embeds in <600ms (5-6ms avg) on M4 Max
- recall@10 unchanged (parity with ONNX baseline)
- 0 GPU heat impact (Metal runs cool)
- Fallback path works on Linux x86_64

## Risk + Mitigation

| Risk | Mitigation |
|---|---|
| Model conversion fails | Pre-convert + ship via HF, no runtime conversion |
| MLX API breaks | Pin mlx-rs version, integration test in CI |
| FP16 precision loss | Run parity test vs ONNX f32, must match >99% |
| User has no GPU | Auto-fallback to ONNX (already designed) |

## Status: ✅ GO — 4-step plan, 200 LOC estimate, weeks 57-65.
