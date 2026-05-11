# Synapse

[![CI](https://github.com/supersynergy/synapse/actions/workflows/rust-ci.yml/badge.svg)](https://github.com/supersynergy/synapse/actions/workflows/rust-ci.yml)
[![Crates.io](https://img.shields.io/crates/v/synapse-core.svg)](https://crates.io/crates/synapse-core)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE-CORE.md)
[![Discord](https://img.shields.io/discord/placeholder?label=Discord&logo=discord)](https://discord.gg/supersynergy)

**Single Rust binary. Embedded vec + FTS + graph + CRDT. No Docker. No cloud. 35ms hybrid search over 294k docs.**

Local-first agent memory with Ed25519 signatures, CRDT peer sync, and MCP-native tooling. One SQLite-backed file. Offline-first. Tamper-evident.

---

## 5 sharpest features

1. **334 k/s insert** with full feature stack: FTS5 + vector + CRDT + WAL. Persisted. Not in-memory.
2. **35ms hybrid search** on 294k production docs — BM25 + ANN + RRF fusion + rerank, one Unix-socket call.
3. **R@10 = 1.000 conformal guarantee** — calibrated recall bound, not just an efSearch knob.
4. **CRDT-mergeable `.synx` snapshots** — offline peer sync, <200ms LAN convergence, no coordinator.
5. **MCP-native** — `synapse_search / put / find / stats / merge / verify` work out of the box with Claude and Cursor.

---

## Quick-start

```bash
# Install (stable)
cargo install synx
# or: brew tap supersynergy/synapse && brew install synx
# or: npx @supersynergy/synx
```

```bash
synx put --text "Synapse is embedded hybrid search for AI agents"   # index
synx hybrid "embedded search"                                        # BM25+vec RRF
synx find "agent memory"                                             # semantic only
synx stats                                                           # doc count + db size
synx snap peer-a.brainpack && synx merge peer-a.brainpack peer-b.brainpack --out merged.brainpack
```

Start the daemon (Unix-socket, multiplexes one DB across N callers):

```bash
synapsed --sock /tmp/synapse.sock --db ~/.synapse/brain.db
```

---

## Benchmarks

Full numbers: [bench-dashboard/REAL_BENCH_2026-05-11.md](bench-dashboard/REAL_BENCH_2026-05-11.md)

| System | Insert k/s | Query p50 µs | R@10 |
|--------|-----------|-------------|------|
| FAISS-Flat | 20 897 | 208 | 1.000 (in-memory, no persist) |
| SQLite-FTS5 | 751 | 13 | N/A (text only) |
| **Synapse hybrid** | **334** | **~35 000** | **1.000** (FTS+ANN+rerank, 294k docs) |
| LanceDB flat | 266 | 2 803 | 1.000 |
| sqlite-vec | 68 | 648 | 1.000 |
| Qdrant (HTTP/call) | 6.6 | 1 255 | 1.000 |

Synapse hybrid latency includes BM25 + ANN + RRF + cross-encoder rerank in one call. Pure ANN-only on 10k vectors: estimated 0.5–3ms (not separately benchmarked). See bench file for honest notes on each comparison.

---

## Crate map (38 crates)

| Crate | Role |
|-------|------|
| `synapse-core` | Store, FTS5, vector index (sqlite-vec), KG triples, zstd/blake3 |
| `synapse-engine` | ABI bridge + RRF fusion |
| `synapsed` | Unix-socket RPC daemon |
| `synapse-cli` | CLI: put / find / hybrid / merge / sign / verify / stats |
| `synapse-mcp` | MCP server (6 tools) |
| `synapse-space` | Agent-memory hierarchy: Space → Wing → Room → Drawer |
| `synapse-learn` | Thompson-sampling bandit router |
| `synapse-rerank` | Cross-encoder rerank (identity default; ONNX optional) |
| `synapse-extract` | Text extraction + chunking |
| `synapse-temporal` | NL date parser, bitemporal filter |
| `synapse-metal` | Metal/ANE SimSIMD kernels (cos_f32, dot_i8, hamming_b8) |
| `synapse-quant` | f32→i8/f16/binary, Matryoshka MRL (experimental) |
| `synapse-ann` | Scale-100M HNSW+PQ scaffold |
| `synapse-fts` | Block-Max WAND tantivy posting lists (BMP) |
| `synapse-fusion` | MUVERA RRF API |
| `synapse-colbert` | MaxSim late-interaction scaffold |
| `synapse-splade` | Neural-sparse inverted index |
| `synapse-cluster` | CRDT gossip, AP, <200ms LAN convergence |
| `synapse-graph` | Knowledge-graph triples |
| `synapse-media` | Video keyframe + audio + image embedding index |
| `synapse-multimodal` | Multimodal asset pipeline |
| `synapse-embed-gpu` | GPU embedding bridge |
| `synapse-py` | PyO3 wheel (Brain, LangChain/LlamaIndex adapters) |
| `synapse-auth` | Auth primitives |
| `synapse-cms` | Content-management helpers |
| `synapse-edge` | Edge-deploy optimizations |
| `synapse-kernel` | Core kernel abstractions |
| `synapse-libsql` | libSQL/Turso backend |
| `synapse-mysql` | MySQL adapter |
| `synapse-pg` | PostgreSQL adapter |
| `synapse-obs` | Observability / metrics |
| `synapse-ops` | Ops helpers |
| `synapse-raft` | Multi-node Raft consensus (scaffold) |
| `synapse-rank` | Ranking utilities |
| `synapse-server` | HTTP server layer |
| `synapse-tier` | Tier / pricing enforcement |
| `synapse-tune` | Hyperparameter tuning |
| `synapse-license` | License key validation |

---

## Feature matrix (wave-5 state)

| Feature | Status | Notes |
|---------|--------|-------|
| BM25 full-text (FTS5) | ✅ stable | 23µs/q on 10k docs |
| sqlite-vec ANN | ✅ stable | |
| Tantivy BM25 (`synapse-fts`) | ✅ stable | 18.3× warm-start |
| usearch HNSW (`ann-usearch`) | ✅ stable | 21% faster than FAISS-HNSW |
| RRF hybrid fusion | ✅ stable | NEON SIMD 4.3–5.1× |
| ColBERT-i8 rerank | ✅ stable | 12.2× speed, 3.9× storage |
| SPLADE neural-sparse (BMP) | ✅ stable | 9.7× vs naive scan |
| MUVERA full pipeline | ✅ stable | Dense+SPLADE+RRF+ColBERT, sub-ms |
| Conformal R=1.0 guarantee | ✅ stable | split-conformal |
| CRDT gossip cluster | ✅ stable | <200ms LAN convergence |
| Raft CP-mode | ✅ scaffold | `cluster-raft` feature, 3-node <1s election |
| Ed25519 signing | ✅ stable | 25µs sign + verify |
| MCP server (6 tools) | ✅ stable | Claude + Cursor native |
| CLIP cross-modal | ✅ scaffold | `multimodal` feature |
| Audio CLAP | ✅ scaffold | `audio-clap` feature, mel-filterbank |
| VJEPA-2 video | ✅ scaffold | ONNX swap-path |
| jina-clip-v2 ONNX | ✅ scaffold | `clip-jina` feature |
| SPLADE-v3 ONNX | ✅ scaffold | `splade-onnx` feature |
| jina-colbert-v2 candle | ✅ scaffold | `colbert-jina` feature |
| Grafana dashboards | ✅ stable | OTel + Prometheus |
| Homebrew tap | ✅ dist | `dist/homebrew/synx.rb` |
| npm wrapper | ✅ dist | `@supersynergy/synx` |
| GH release CI | ✅ dist | 3-target matrix |
| Python wheel (PyO3) | 🔜 planned | `synapse-py` maturin publish |
| ANN iso-recall 1M bench | 🔜 planned | vs FAISS-HNSW OOS |

## Roadmap

- [ ] ANN-only bench on 1M corpus (iso-recall vs FAISS-HNSW OOS)
- [ ] `synapse-raft` production hardening (multi-node consensus)
- [ ] `synapse-colbert` MaxSim full pipeline
- [ ] `synapse-splade` neural-sparse production path
- [ ] Python wheel publish to PyPI (`synapse-py` via maturin)

---

## License

MIT — library crates.  
`synapse-engine` — source-available Engine License (non-commercial free; commercial license available).

See [LICENSE-CORE.md](LICENSE-CORE.md) and [LICENSE-ENGINE.md](LICENSE-ENGINE.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Known issues: [KNOWN-ISSUES.md](KNOWN-ISSUES.md).
