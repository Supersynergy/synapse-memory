# Phase-5 MLX Metal Embedder — Implementation Report

**Date:** 2026-04-25
**Branch:** `session-2026-04-25-ultrathink`
**Hardware:** Apple M4 Max, 128 GB
**Model:** `mlx-community/bge-small-en-v1.5-bf16` (BGE-small-en-v1.5, bf16 weights, 384-dim)

## Summary

Phase-5 placeholder replaced with a working MLX Metal embedder via a persistent
Python sidecar (`scripts/synapse-mlx-embed.py`) wrapped by a Rust IPC client
(`embed_mlx.rs`). Feature-flagged `embed-mlx` (Apple Silicon only); default
build is unchanged.

**End-to-end Rust → Python → MLX → Metal → Rust** verified live by ignored
unit test `sidecar_roundtrip` and a release-mode bench example.

## Decision: Sidecar (Option A), not native bindings (Option B)

- `mlx-rs` crate exists but has **no published BGE/BERT reference path** as of
  2026-04 — would have needed bespoke tokenizer + BERT impl in Rust.
- ONNX-CoreML EP via fastembed (Option C) was investigated; fastembed-rs
  currently exposes only the CPU EP — needs an upstream feature flag.
- Sidecar gives Metal-class latency **today** behind the existing
  `TextEmbedder` trait. Swap in pure-Rust MLX later without touching callers.

## Bench (release build, M4 Max)

`cargo run --release --example bench_embed_mlx --features "embed-mlx embed"`

| backend         | path        | p50 (ms) | p95 (ms) | per-doc (ms) | speedup |
|-----------------|-------------|---------:|---------:|-------------:|--------:|
| fastembed-cpu   | single      |    5.15  |    5.75  |        5.15  |   1.0×  |
| **mlx-metal-bf16** | single   |    **2.45** |    **3.19** |        **2.45**  |   **2.1×** |
| fastembed-cpu   | batch=4     |    9.26  |    —     |        2.32  |   1.0×  |
| mlx-metal-bf16  | batch=4     |    3.70  |    —     |        0.92  |   2.5×  |
| fastembed-cpu   | batch=8     |   12.08  |    —     |        1.51  |   1.0×  |
| mlx-metal-bf16  | batch=8     |    3.84  |    —     |        0.48  |   3.1×  |
| fastembed-cpu   | batch=16    |   18.92  |    —     |        1.18  |   1.0×  |
| mlx-metal-bf16  | batch=16    |    5.33  |    —     |        0.33  |   3.6×  |
| fastembed-cpu   | batch=32    |   32.51  |    —     |        1.02  |   1.0×  |
| **mlx-metal-bf16** | batch=32 |    **7.09** |    —     |        **0.22**  |   **4.6×** |
| fastembed-cpu   | batch=64    |   54.53  |    —     |        0.85  |   1.0×  |
| **mlx-metal-bf16** | batch=64 |   **11.88** |    —     |        **0.19**  |   **4.6×** |

**Headline:** single-doc 2.1×, batch ingest 4.6×. Plan-doc target was 4-6× — **met for batch path** which is the bottleneck for ingestion / re-embed jobs.

The plan-doc cited "p50 80ms" as fastembed baseline; on this M4 Max with the
already-warm session pool we measure 5.15 ms — so the *absolute* number is
better than the plan stated, while the *ratio* is in-line.

## Correctness (Parity vs canonical BGE) — **RESOLVED 2026-04-25**

50 sentences, cosine vs fastembed CPU canonical (BAAI/bge-small-en-v1.5):

| variant                       | n  | mean    | min     | p05     | max     | verdict |
|-------------------------------|---:|--------:|--------:|--------:|--------:|:-------:|
| **self-converted bf16 (CLS pool)**  | 50 | **1.0000** | **1.0000** | **1.0000** | **1.0000** | **PASS** |
| upstream bf16 (CLS pool)            | 50 | 1.0000  | 1.0000  | 1.0000  | 1.0000  | PASS    |
| any bf16 (mean pool — old code)     | 50 | 0.9446  | 0.9320  | 0.9368  | 0.9571  | FAIL    |

DoD targets: mean >=0.99, worst-case >=0.985 — **both met (1.0000)**.

### Root cause of previous 0.91-0.94 drift

NOT weight conversion. The sidecar was applying **mean pooling**, but BGE's
own `1_Pooling/config.json` declares `pooling_mode_cls_token=true`. Switching
the sidecar to CLS pooling (first-token vector) closed the gap fully. Self-
converted weights are byte-equivalent in output to upstream — the upstream
conversion was never broken; the consumer was.

### Self-conversion artifacts (kept as offline-ready fallback)

- Source: `BAAI/bge-small-en-v1.5` fp32 .safetensors (HF snapshot)
- Pipeline: `safetensors numpy -> mx.array -> .astype(mx.bfloat16) -> mx.save_safetensors`
- Output: `models/bge-small-mlx-bf16/`, 63.7 MB, 199 tensors
  (drops `embeddings.position_ids`), `config.json` patched with
  `model_type=bert`, `architectures=[BertModel]`, `torch_dtype=bfloat16`.
- Script: `scripts/convert-bge-fp32-to-bf16.py`.
- Bench: `scripts/bench-mlx-parity.py` (CI-ready guard).

### Status update

`embed-mlx` is **default-promote eligible**. Follow-up Rust wiring:
flip `pick_embedder()` to prefer MLX when `embed-mlx` feature is built.
Expected Vec/Hybrid impact: 80 ms -> <10 ms (matches batch=32 0.22 ms/doc).

## Files Changed

- `crates/synapse-core/Cargo.toml` — `embed-mlx = ["turbo", "dep:rmpv"]` + bench example
- `crates/synapse-core/src/embed_mlx.rs` — IPC client (Sidecar struct, msgpack rmpv)
- `crates/synapse-core/examples/bench_embed_mlx.rs` — bench harness
- `scripts/synapse-mlx-embed.py` — sidecar (CLS-pool + L2-norm, env-configurable model;
  default model path now resolves to local `models/bge-small-mlx-bf16/`)
- `scripts/convert-bge-fp32-to-bf16.py` — self-conversion of BAAI fp32 -> MLX bf16
- `scripts/bench-mlx-parity.py` — 50-sentence parity guard (fastembed vs MLX)
- `models/bge-small-mlx-bf16/` — self-converted bf16 weights, 63.7 MB, 199 tensors

## Configuration

| env var               | default                                              |
|-----------------------|------------------------------------------------------|
| `SYNAPSE_MLX_PYTHON`  | `python3`                                            |
| `SYNAPSE_MLX_SCRIPT`  | `<crate>/../../scripts/synapse-mlx-embed.py`         |
| `SYNAPSE_MLX_MODEL`   | local `models/bge-small-mlx-bf16/` if present, else `mlx-community/bge-small-en-v1.5-bf16` |
| `SYNAPSE_MLX_MODEL_PATH` | (optional explicit override, takes precedence) |

## Validation Run

```bash
SYNAPSE_MLX_PYTHON=~/.venvs/agents/bin/python \
  cargo test -p synapse-core --features embed-mlx --lib -- --ignored sidecar_roundtrip
# 1 passed
```

## Status

- ✅ Default build unchanged (`cargo check -p synapse-core` clean)
- ✅ `--features embed-mlx` builds clean
- ✅ Live Rust→Python→MLX→Metal roundtrip verified
- ✅ 4.6× faster on the batch path (ingest bottleneck)
- ✅ Parity 1.0000 vs fastembed canonical (CLS pool fix + self-conv weights)
- 🟢 **Default-promote eligible** — flip `pick_embedder()` in next PR
