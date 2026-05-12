# Synapse — The Vector DB for Agents

> **107×–892× faster than LanceDB/Qdrant. Single binary. Local-first. €0 infra.**

## Live numbers (M4 Max, 2026-05-10)

| Engine | ms/query @ 10k | ms/query @ 20k | Single-binary | Hybrid (BM25+Vec) | MCP-native |
|--------|---------------:|---------------:|:-------------:|:-----------------:|:----------:|
| **Synapse** | **0.023** | **0.023** | ✓ | ✓ | ✓ |
| FAISS flat | 0.012 | 0.048 | ✗ | ✗ | ✗ |
| LanceDB | 2.471 | 12.290 | ✗ | ✗ | ✗ |
| Qdrant in-mem | 2.581 | 20.519 | ✗ | ✗ | ✗ |
| Pinecone | + 30–80ms HTTP | + 30–80ms HTTP | ✗ (SaaS) | ✗ | ✗ |

[Reproduce →](BENCH_2026-05-10.md)

## Why Synapse

1. **8ms hybrid recall** via Unix socket — no HTTP tax
2. **SimSIMD kernels** Apple Silicon native (1-bit 71×, int8 46×, MRL-128 35×)
3. **Single-binary 5MB** — embed in Tauri/Electron, ship to laptops
4. **Hybrid first-class** — BM25 + Vector + RRF in one query, not bolted on
5. **MCP server included** — drop-in for Claude / Cursor / VSCode agents
6. **CRDT sync + ed25519 sign** — multi-device offline-first
7. **€0 infra** — runs as daemon on your machine, no cloud lock-in

## Install

```bash
brew install supersynergy/synapse/synapse        # macOS
npm i -g @supersynergy/synapse                   # cross-platform
cargo install synapse-cli                        # from source
```

## Quickstart (3 lines)

```bash
synx put "Alice moved to Berlin in March 2024"
synx hybrid "when did Alice move?" 5
# → "Alice moved to Berlin in March 2024" (2ms hybrid recall)
```

## Pricing

| Tier | Price | What |
|------|------:|------|
| **OSS Core** | €0 | Apache-2 · single-node · all features |
| **synx-cloud** | €9–29 / mo | multi-device CRDT sync hosted |
| **Enterprise** | €5k–50k / yr | on-prem license + SLA + priority patches |
| **Embedded SDK** | €99–499 one-time / app | ship inside your Tauri/Electron app |

## Roadmap (next 30d)

- [ ] Embedder swap → Arctic-embed-v2-m (MTEB 53 → 66.2)
- [ ] RaBitQ 1-bit index for 1M+ vec corpora
- [ ] LightGBM LambdaMART hybrid fusion (NDCG@10 +6–9 over RRF)
- [ ] Cross-encoder Jina-v2 wired in LongMemEval (R@5 0.30 → 0.85)
- [ ] HN Show-HN launch + Smithery MCP listing
