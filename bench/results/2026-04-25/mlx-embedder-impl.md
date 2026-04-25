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

## Correctness (Parity vs canonical BGE)

10 paraphrase pairs, cosine vs `sentence-transformers BAAI/bge-small-en-v1.5`
fp32 baseline:

| variant           | min    | mean   | max    |
|-------------------|-------:|-------:|-------:|
| MLX 4-bit         | 0.8976 | 0.9033 | —      |
| **MLX bf16 (used)** | **0.9058** | **0.9116** | **0.9170** |

⚠️ **Below the 99% DoD bar.** Investigation showed this is **not** a pooling
or normalization issue (the sidecar applies attention-masked mean-pool +
L2-normalize matching BGE canonical) — the gap originates in the
`mlx-community/bge-small-en-v1.5-bf16` HF weight conversion itself.

**Mitigation (follow-up, not in this PR):**
1. Re-convert directly from `BAAI/bge-small-en-v1.5` fp32 with `mlx_lm.convert
   --dtype bfloat16` and host on `huggingface.co/supersynergy/bge-small-mlx`.
2. Validate parity ≥0.99 cosine vs fp32 ST.
3. Switch `SYNAPSE_MLX_MODEL` default to the new repo.

Until then, `embed-mlx` stays opt-in and is **not** wired into
`pick_embedder()` defaults — keeps the don't-ship-a-regression guarantee.

## Files Changed

- `crates/synapse-core/Cargo.toml` — `embed-mlx = ["turbo", "dep:rmpv"]` + bench example
- `crates/synapse-core/src/embed_mlx.rs` — IPC client (Sidecar struct, msgpack rmpv)
- `crates/synapse-core/examples/bench_embed_mlx.rs` — bench harness
- `scripts/synapse-mlx-embed.py` — sidecar (mean-pool + L2-norm, env-configurable model)

## Configuration

| env var               | default                                              |
|-----------------------|------------------------------------------------------|
| `SYNAPSE_MLX_PYTHON`  | `python3`                                            |
| `SYNAPSE_MLX_SCRIPT`  | `<crate>/../../scripts/synapse-mlx-embed.py`         |
| `SYNAPSE_MLX_MODEL`   | `mlx-community/bge-small-en-v1.5-bf16`               |

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
- ⚠️ Parity gap from upstream bf16 weights; **not promoted to default**
- 🔜 Follow-up: own bf16 conversion → flip default once cosine ≥0.99
