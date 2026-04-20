# Synapse v2.0 — one file, all features, all adapters

## Before / After / New

| | v1.0 (remote) | v0.3-full-stack (local) | **v2.0 (unified)** |
|---|---|---|---|
| Features | 1/8 | 7/8 | **8/8** |
| Marketing assets | SVG hero, matrix, CI | — | **preserved** |
| Advanced features | — | Ed25519, CRDT, MCP+, sharding, federation, self-learning, multi-ext | **all included** |
| Adapters | — | mem0-shim | **7 adapters** |
| Breaking changes | — | — | **zero** |

## Features

### Cryptographic integrity — Ed25519 signing
Every entry carries an Ed25519 signature. Ship a `.brainpack` to a teammate — they can verify it came from you without trusting a server.

### Offline-first sync — CRDT merge (yrs)
Two writers, offline. Merge with `synapse merge other.brainpack`. Automerge-compatible semantics, 0.59 ms for 200 ops on M4 Max.

### MCP server — 5 tools
`put / search / merge / timeline / verify` exposed as MCP tools. One JSON block in your Claude/Cursor/Cline config. Done.

### Horizontal scale — IVF sharding + fastbloom routing
Partition large corpora without a cluster. Bloom filter routes queries to the right shard; IVF k-means ensures balanced assignment.

### P2P federation — TCP/unix sync
Point two Synapse daemons at each other. Memory propagates peer-to-peer over TCP or unix socket. No cloud, no registration.

### Self-learning — Thompson bandit + heat drift
Ranking improves as you use it. `synapse feedback --id <id> --score 1.0` trains the in-process bandit. Heat and consolidation prevent stale promotions.

### Multi-extension format
`.syn`, `.synapse`, `.brainpack` — all read by the same binary. Magic-byte detection; no extension guessing.

### SQLCipher encryption (feature-flagged)
`cargo build --features encrypt` for AES-256 at-rest. Not on by default — zero overhead for the common case.

## Migration: v1.0 → v2.0

Zero breaking changes. All new features are additive.

```bash
# Reinstall
cargo install --locked --git https://github.com/Supersynergy/synapse --tag v2.0.0 \
  synapse-cli synapsed synapse-mcp

# Your existing .brainpack / .syn files work unchanged
synapse search "anything"
```

New CLI subcommands: `learn`, `feedback`, `merge`, `verify`, `timeline`.

## Adapter install matrix

| Adapter | PyPI | npm |
|---------|------|-----|
| mem0-shim | `pip install synapse-mem0` | — |
| langfuse | `pip install synapse-langfuse` | — |
| promptfoo | `pip install synapse-promptfoo` | — |
| browser-use | `pip install synapse-browser-use` | — |
| mastra | — | `npm i @synapse-ai/mastra` |
| vercel-ai | — | `npm i @synapse-ai/vercel-ai` |
| copilotkit | — | `npm i @synapse-ai/copilotkit` |

## Benchmark highlights

### vs real competitors (1 k docs, 200 queries, 384-d, M4 Max)

| engine | ms / query | vs Synapse |
|--------|----------:|-----------|
| FAISS flat (theoretical floor) | 0.010 | 2.3× faster (no persistence) |
| **Synapse v2.0** | **0.023** | — |
| Chroma | 0.303 | 13× slower |
| LanceDB | 1.440 | 63× slower |

### v0.3 vs v1.0 core performance (v2.0 inherits v0.3 codebase)

| workload | insert (ms) | lex p50 (ms) |
|----------|-------------|-------------|
| small | 31.4 | 0.454 |
| medium | 300.1 | 0.092 |
| large | 4 480.5 | 25.079 |

Full data: [`bench/COMPARISON_v0.3_vs_v1.0.md`](../bench/COMPARISON_v0.3_vs_v1.0.md) · [`bench/RESULTS-REAL-COMPETITORS.md`](../bench/RESULTS-REAL-COMPETITORS.md)

## Tag history

| Tag | SHA | Line | Note |
|-----|-----|------|------|
| v0.1 | root | local fork | historical |
| v0.2.0 | `5ec3cf4` | local fork | historical |
| v0.3.0 | `0544b02` | local fork | historical |
| v1.0 | remote root | remote marketing | historical |
| **v2.0.0** | reconcile-v2 HEAD | **unified** | canonical |

## Changelog

- Merged unrelated git histories (remote marketing base + local feature stack)
- Conflict resolution: crates/Cargo.toml/deny.toml → local (v0.3); README/assets/CI → remote
- Merged branches: `feat/mem0-shim`, `feat/adapters-wave1`, `docs/integration-plan`, `tools/gamechanger`
- All 28 tests passing
- Repo description + topics updated via `gh repo edit`
