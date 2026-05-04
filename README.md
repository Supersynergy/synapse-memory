# Synapse

Fastest reliable embedded vector+FTS+KG database on Apple Silicon — single Rust binary, no external services.

## Hard Targets (from RESULTS.md)

| Metric | Target | Actual |
|--------|--------|--------|
| FTS query p50 | ≤ 0.05 ms | **0.051 ms** ✓ |
| Insert throughput | ≥ 4 760 ops/s (Python adapter) | **4 528–4 760 ops/s** ✓ |
| vs ChromaDB insert | 2× | **2.9×** ✓ |
| vs ChromaDB query | 7× | **7.7×** ✓ |
| Storage overhead | ≤ 2× sqlite-vec | **0.9×** ✓ |
| vs FAISS query | ≤ 3× floor | **2.3×** ✓ |

See [RESULTS.md](RESULTS.md) for full bench data.

## Install

```bash
cargo install --path crates/synapse-cli
```

Or download a release binary from [Releases](https://github.com/Supersynergy/synapse/releases).

## Quickstart

```bash
# 1. Put a document
synapse put --text "Synapse is fast embedded memory for AI agents"

# 2. Full-text search
synapse find "embedded memory"

# 3. Hybrid search (BM25 + vector RRF)
synapse hybrid "fast AI memory"

# 4. Stats
synapse stats

# 5. Export signed snapshot
synapse keygen --out key.bin
synapse snap-signed --key key.bin --out memory.brainpack
```

## Architecture — 17 Crates

```
synapse-core      Store, FTS5, vector index (sqlite-vec), KG triples, zstd/blake3
synapse-engine    Hybrid query planner, RRF fusion, cache
synapse-space     Agent-memory: Space → Wing → Room → Drawer hierarchy
synapsed          Unix-socket RPC daemon (/tmp/synapse.sock)
synapse-cli       CLI binary (synapse)
synapse-mcp       MCP server (synapse_search, synapse_put, synapse_find, synapse_stats)
synapse-learn     Bandit router (Thompson sampling), per-query calibration
synapse-rerank    Cross-encoder rerank via ONNX runtime
synapse-extract   Text extraction + chunking (per-message, fixed-window, semantic)
synapse-temporal  Temporal KG: validity ranges, bitemporal filter
synapse-metal     Metal/ANE SimSIMD kernels (cos_f32, dot_i8, hamming_b8)
synapse-ann       Scale-100M ANN scaffold (HNSW + PQ, stub)
synapse-quant     Quantisation: f32→i8/f16/binary, Matryoshka MRL
synapse-wal       Crash-safe bulk-ingest write-ahead-log helpers
synapse-seg       Segment/shard management (>10M chunks)
synapse-license   License key validation
synapse-py        PyO3 Python wheel (synapse.Brain, LangChain/LlamaIndex integration)
```

## Running the Daemon

The daemon multiplexes a single DB file across multiple callers over a Unix socket.

```bash
# Start manually
synapsed --sock /tmp/synapse.sock --db ~/.synapse/brain.db

# macOS LaunchAgent (auto-start, keepalive)
cp scripts/synapsed-launchd.plist ~/Library/LaunchAgents/com.supersynergy.synapsed.plist
launchctl load ~/Library/LaunchAgents/com.supersynergy.synapsed.plist

# Logs
tail -f ~/.synapse/synapsed.log
```

## Python Wheel

```bash
cd crates/synapse-py
maturin develop
python -c "import synapse; b = synapse.Brain(); b.put('hello'); print(b.hybrid('hello'))"
```

## Benchmarks

```bash
# FTS + hybrid bench (criterion)
cargo bench -p synapse-core

# MemPalace shootout vs ChromaDB
cd bench/mempalace-shootout && python run.py

# Full results
cat RESULTS.md
```

## Known Issues

See [KNOWN-ISSUES.md](KNOWN-ISSUES.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
