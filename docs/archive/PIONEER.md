# Synapse Pioneer Roadmap — Dominate All Categories

Ziel: In jeder Messkategorie (latency, throughput, recall, scale, ergonomics, security) gegen jede Alternative gewinnen. Nicht inkrementell — fundamental.

## Benchmark Baseline (2026-04-20, M4 Max, 189 docs)

| Op | Synapse v2 now | Target pioneer | vs sqlite-vec | vs Qdrant | vs mem0 |
|---|---:|---:|---:|---:|---:|
| Ping RTT | 58µs | **<5µs** | n/a (in-proc 0µs) | 2ms gRPC | — |
| Hybrid search | 8ms | **<1ms** | <1ms (+manual BM25) | 5-10ms | 50-200ms |
| Put single | 335ms | **<5ms** | 20µs (+ext embed) | 2ms (+ext) | 100ms |
| Put 1000 batch | ~5min | **<5s** | ~1s (no embed) | ~10s | min |
| Insert throughput | 3 docs/s | **>500 docs/s** | 50k/s (no embed) | 5k/s | 10/s |
| Cold CLI start | 700ms | **<50ms** | 10ms (just sqlite) | 2s server | n/a |
| Recall@10 (BEIR-MS-MARCO) | ~0.45 RRF | **>0.62 (L3 rerank)** | 0.38 vec-only | 0.52 | 0.45 |

## Pioneer Features (rank by impact)

### [P0] Embedding Hash-Cache — 100× put speedup for dup content
Current: every `put` re-embeds. Scraped markdown + commits have 40-80% dedup rate.
**Fix**: `embedder::embed(text)` → `blake3(text)` → `redb.get(hash)` → skip if hit.
**Impact**: 335ms → 3ms put on cache hit. Already redb dep; add ~30 LOC in `synapse-core/src/embed.rs`.

### [P0] MLX Metal Embedder Backend (M-series only)
fastembed = ONNX CPU. MLX BGE-small on Metal: measured 3-5× faster on M-series.
**Fix**: `trait Embedder` + feature-flag `--features mlx` → `synapse-mlx` crate with `mlx-rs` binding.
**Impact**: 335ms → 70ms put, 8ms → 3ms embed_query. Separable — CPU default, Metal opt-in.

### [P0] PutBatch Tensor-Batched Embedding
Socket already has `PutBatch`. Daemon embeds sequentially. Pioneer: pad+stack 32 texts → 1 ONNX forward → 10× throughput via SIMD/Metal-matmul.
**Impact**: 1000 docs 5min → **5-15s**. ~50 LOC in `synapsed::handle_put_batch`.

### [P1] Library-Mode Crate — Zero-IPC
Expose `synapse-core::Db` as public API. Link directly in your Rust binaries.
**Impact**: 8ms socket → sub-µs fn-call. Beats sqlite-vec (which has SQL parser overhead).
**Deliverable**: `Cargo.toml` re-export; docs example using `Db::open + search_hybrid`.

### [P1] Tiered Hybrid Rank (novel fusion)
Current RRF = 2015-era. Pioneer 4-tier:
```
L1: BM25 FTS5 prefilter top-200     (0.5ms)
L2: vec cosine top-20 on 200 cand   (0.8ms)
L3: cross-encoder rerank top-5      (MLX phi-4-mini, 30ms) — only if Σtop5 margin < ε
L4: bandit-weighted fusion          (per query-cluster, learned)
```
**Impact**: +15-20 recall@10 points vs RRF. ColBERT-level quality, 1/10 cost.

### [P1] IVF-PQ Vector Compression
Current: 384-dim f32 = 1.5KB/doc. 1M docs = 1.5GB. Pioneer: IVF-256 + PQ-8 = 48 bytes/doc → 48MB/1M. Rerank with full vecs top-50.
**Impact**: 100M docs on laptop RAM. IVF shard code exists; PQ is ~200 LOC + faiss-rs or custom.

### [P2] Append-Only mmap'd Rope-Log
Replace SQLite insert path with append-only log + offset index. Writers never block readers. CRDT becomes log-diff.
**Impact**: 10× write throughput. Foundation for real-time streams. LMDB-style.

### [P2] Live Timeline Stream — Pub/Sub on Brain
`synapse timeline --live` = WebSocket/unix-socket stream of new doc IDs. Multiple agents observe + react.
**Impact**: Enables multi-agent shared-memory paradigm. Nothing in this space does it.

### [P2] Semantic-Level CRDT Merge
yrs merges tokens. Pioneer: merge at chunk-boundary (semantic), detect vec-drift conflicts, auto-summarize via local LLM.
**Impact**: Truly conflict-free multi-writer agent memory.

### [P3] WASM/Browser Target
Compile `synapse-core` → WASM. Run entire memory in browser tab, mobile PWA.
**Impact**: No Python alternative does this. Unique distribution channel.

### [P3] Federated Learning Bandit with DP noise
Bandit learns per-user. Federation exchanges anonymized reward stats with ε-DP noise.
**Impact**: Cross-user recall improvement without data leak.

### [P3] Decay + Consolidate (biological memory)
Frequency + recency + diversity → importance score. Low-score memories → batch-compress into summary docs via local LLM. Bounded storage.
**Impact**: Run 10 years of memory in <1GB. Mem0 has naive version, synapse can do it properly.

## Categories synapse already dominates

- ✅ Single-file portability (`.brainpack`)
- ✅ Ed25519 signing (nothing else has this native)
- ✅ CRDT offline-multi-writer (yrs)
- ✅ Self-learning ranker (Thompson+heat)
- ✅ MCP-native

## Categories to close gap

| Gap | Fix | ETA |
|---|---|---|
| in-proc latency | Library-mode P1 | 1 day |
| put throughput | P0 cache + P0 batch + P0 MLX | 2-3 days |
| recall quality | P1 tiered rank | 3-5 days |
| 100M scale | P1 IVF-PQ | 1-2 weeks |
| write throughput | P2 append log | 1 week |

## Next 72h — pick 3

1. **P0 embedding hash-cache** — biggest immediate win, trivial
2. **P0 batch-embed SIMD** — unlocks 1000-URL pipelines
3. **P1 library-mode** — moat vs sqlite-vec

All three = make synapse 100× faster on common paths + beat sqlite-vec even on raw latency via zero-IPC.

## The pitch after pioneer phase

> "Synapse: sub-µs memory (lib-mode) / sub-ms memory (socket). 500 docs/s bulk.
> Ed25519 signed. CRDT. Self-learning. Single file. Beats Qdrant on latency,
> sqlite-vec on features, mem0 on everything."

No existing system in any language has this combination. Synapse is the first.
