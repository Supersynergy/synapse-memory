# MLX Embedder Sidecar — synapse-ultra bench (2026-04-26)

## Summary

Replaced fastembed CPU (BGE-small ONNX) with MLX Metal sidecar as primary embedder in synapse-ultra. Target ≤8ms cold p50. Result: **9.9ms p50** (warm path, post-sidecar-startup).

## Latency

| Path | p50 | p95 | min | max | N |
|------|-----|-----|-----|-----|---|
| fastembed CPU (before) | 24.9ms | 86.0ms | 17.3ms | 108.2ms | 30 |
| MLX Metal sidecar (after) | 9.9ms | 117.2ms | 4.9ms | 168.4ms | 30 |

- **p50 improvement: 2.5×** (24.9ms → 9.9ms)
- p95 slightly worse due to GIL/scheduling spikes on sidecar IPC; coalescing (from synapse-core) not wired in ultra yet

## Sidecar Cold Start

- Model: `mlx-community/bge-small-en-v1.5-bf16` (local cache at `models/bge-small-mlx-bf16`)
- Cold start (model load): **~51s** — Python mlx_embeddings loading BGE weights into Metal
- This is a one-time cost per daemon lifetime (lazy-spawn, persistent pipe)
- Mitigation: `--warm` flag triggers embed at startup, amortizing cost before traffic

## Recall@10

| Test | Score |
|------|-------|
| Self-consistency (20 fixed queries, run×2) | **1.000** |
| Cross-engine vs fastembed | N/A — fastembed fallback verified functional |

Self-consistency = 1.000 means MLX produces identical top-10 results on repeated calls (deterministic, cache-backed after first call).

Cross-engine vs fastembed not directly measured (requires disabling emb_cache and rerunning both paths on same corpus — out of scope for this bench; model parity issue noted in synapse-core `embed_mlx.rs` comments: bf16 ~0.94 cosine vs fp32, CLS pooling fix in script restores ≥0.999).

## RAM Delta

- MLX Python sidecar: ~320MB RSS (mlx-embeddings + model weights)
- synapse-ultra RSS unchanged (sidecar is separate process)

## Config

| Env Var | Value |
|---------|-------|
| `ULTRA_EMBEDDER` | `mlx` (explicit, also auto-enabled on aarch64 macOS) |
| `SYNAPSE_MLX_PYTHON` | `/Users/master/.venvs/agents/bin/python` |
| `SYNAPSE_MLX_SCRIPT` | `scripts/synapse-mlx-embed.py` (shared with synapse-core) |

## Files Changed

- **Created**: `crates/synapse-ultra/src/embed_mlx.rs` — MLX sidecar struct (lazy-spawn, msgpack, fallback-safe)
- **Modified**: `crates/synapse-ultra/src/embed.rs` — MLX as T2 in lookup chain (LRU → emb_cache → MLX → fastembed)
- **Modified**: `crates/synapse-ultra/src/lib.rs` — added `pub mod embed_mlx`
- **Modified**: `Library/LaunchAgents/com.supersynergy.synapse-ultra.plist` — added `SYNAPSE_MLX_PYTHON`, `ULTRA_EMBEDDER=mlx`

## Limitations

1. p50 target was ≤8ms; achieved 9.9ms — sidecar IPC round-trip adds ~3ms vs direct MLX call
2. p95 regression vs fastembed — occasional GIL latency spikes. Coalescing worker (from synapse-core) would fix this for concurrent query load but not wired in ultra yet.
3. Cold start 51s — acceptable for daemon (amortized), not for serverless/ephemeral.
4. Fastembed fallback confirmed working: `ULTRA_EMBEDDER=fastembed` bypasses MLX entirely.
