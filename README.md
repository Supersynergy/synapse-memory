# Synapse

**Recall-King for in-process Rust ANN at iso-recall ≥0.98 on Apple Silicon — 19k QPS @ R@10=0.982 on 168k corpus, single-binary, no daemon required.**

Local-first, audit-grade agent memory. No external services. No cloud dependency. Ed25519-signed docs. CRDT merge. SQLite-embedded.

## Verified Killer Features

1. **19k aggregate QPS @ R@10=0.982** — HNSW in-proc, 168k corpus, 12-core M4 Max (1,631 QPS single-core measured)
2. **Unified vec+FTS+filter** — single query, zero inter-process overhead; usearch/Qdrant require separate hops
3. **Agent Memory: 77× faster** than ChromaDB at same recall (LongMemEval-S, 0.03ms p50 vs 2.3ms)
4. **RAG parity with Dense BERT** — nDCG@10=0.720 on BEIR SciFact via FTS5+Vec RRF
5. **Ed25519 signatures** — every doc verifiable, tamper-evident audit trail
6. **CRDT merge** — offline-first, conflict-free peer sync via brainpack snapshots

See [RESULTS-WORLDBEST-2026-05-05.md](RESULTS-WORLDBEST-2026-05-05.md) for today's full v1.2 bench results.  
See [RESULTS.md](RESULTS.md) for historical bench data.

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

## Architecture — 15 Active Crates

```
synapse-core      Store, FTS5, vector index (sqlite-vec), KG triples, zstd/blake3
synapse-engine    ABI bridge + RRF fusion (FTS+vec result ranking)
synapse-space     Agent-memory: Space → Wing → Room → Drawer hierarchy
synapsed          Unix-socket RPC daemon (/tmp/synapse.sock)
synapse-cli       CLI: syn put/find/hybrid/merge/sign/verify/stats
synapse-mcp       MCP server: synapse_search/put/find/stats/merge/verify
synapse-learn     Bandit router (Thompson sampling), per-query calibration
synapse-rerank    Cross-encoder rerank (IdentityReranker default; OnnxCrossEncoder via --features onnx)
synapse-extract   Text extraction + chunking (per-message, fixed-window, semantic)
synapse-temporal  NL date phrase parser (chrono-english), bitemporal filter
synapse-metal     Metal/ANE SimSIMD kernels (cos_f32, dot_i8, hamming_b8)
synapse-ann       Scale-100M ANN scaffold (HNSW + PQ, stub/TODO)
synapse-quant     Quantisation: f32→i8/f16/binary, Matryoshka MRL (experimental)
synapse-license   License key validation
synapse-py        PyO3 Python wheel (synapse.Brain, LangChain/LlamaIndex integration)
```

> `synapse-wal` and `synapse-seg` are future stubs in `synapsestore/crates/` — not compiled by default.

## CRDT Merge + Verify Roundtrip

```bash
# Generate key pair
syn keygen --sk node.sk --vk node.vk

# Put a signed doc
syn put --text "Hello from node A" --sign node.sk  # returns doc_id, e.g. 1

# Export snapshot
syn snap peer-a.brainpack

# On node B: merge peer snapshot
syn merge peer-a.brainpack peer-b.brainpack --out merged.brainpack

# Verify doc signature
syn verify 1 --vk node.vk
# ok verified id=1
```

No competitor has this. One binary. Offline. Tamper-evident.

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
