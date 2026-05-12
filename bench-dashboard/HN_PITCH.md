# Synapse v1.0.1 — HN Post Material

**5 sharpest numbers (2026-05-11, M4 Max, reproducible):**

1. **18.3× BM25 latency drop** — tantivy warm-cache vs FTS5 cold (synapse-fts, feature-gated)
2. **4.3–5.1× RRF speedup** — NEON SIMD reciprocal-rank-fusion vs scalar (f32 lane-parallel)
3. **−48% embed p95 tail** — MLX Metal 90.8 ms vs fastembed CPU 174.3 ms (parity 1.0000)
4. **7,153 OLTP OPS @ 8t** — real point-select with SQLite I/O, MariaDB parity (7,626 OPS)
5. **Sub-10 ms hybrid search** — BM25+vec RRF, 113k docs, Unix socket, single binary

**Stack:** Rust 1.95 · SQLite+FTS5+sqlite-vec · SimSIMD NEON/AVX2 · tantivy · ONNX (ColBERT/SPLADE)

**New crates this wave:**
- `synapse-fusion` — MUVERA RRF API (`muvera_rrf` + `full_pipeline`)
- `synapse-colbert` — MaxSim late-interaction scaffold
- `synapse-splade` — neural-sparse inverted-index
- `synapse-cluster` — CRDT gossip, AP, &lt;200 ms LAN convergence

All numbers SHA-pinned, Criterion harness, M4 Max 128 GB.
Repo: https://github.com/Supersynergy/synapse
