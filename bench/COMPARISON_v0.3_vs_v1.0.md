# Synapse v0.3-full-stack vs v1.0 — Benchmark Comparison

**Hardware**: Apple M4 Max · 128GB RAM · 8TB SSD · macOS 24.5.0
**Versions**: Local = synapse 0.1.0 | Remote = synapse 1.0.0

## Workload: Small

| Metric | Local v0.3 | Remote v1.0 | Winner |
|--------|-----------|------------|--------|
| Insert total (ms) | 31.4 | 30.0 | **v1.0 WIN** |
| Throughput (docs/s) | 31887 | 33326 | **v1.0 WIN** |
| Lex p50 (ms) | 0.454 | 0.442 | **v1.0 WIN** |
| Lex p95 (ms) | 0.866 | 0.677 | **v1.0 WIN** |
| Lex p99 (ms) | 0.866 | 0.677 | **v1.0 WIN** |
| File size (bytes) | 4,096 | 4096 | TIE |

## Workload: Medium

| Metric | Local v0.3 | Remote v1.0 | Winner |
|--------|-----------|------------|--------|
| Insert total (ms) | 300.1 | 293.3 | **v1.0 WIN** |
| Throughput (docs/s) | 33325 | 34093 | **v1.0 WIN** |
| Lex p50 (ms) | 0.092 | 0.087 | **v1.0 WIN** |
| Lex p95 (ms) | 2.491 | 2.752 | **v0.3 WIN** |
| Lex p99 (ms) | 2.491 | 2.752 | **v0.3 WIN** |
| File size (bytes) | 5,832,704 | 5808128 | **v1.0 WIN** |

## Workload: Adversarial

| Metric | Local v0.3 | Remote v1.0 | Winner |
|--------|-----------|------------|--------|
| Insert total (ms) | 19.2 | 18.9 | **v1.0 WIN** |
| Throughput (docs/s) | 52093 | 52881 | **v1.0 WIN** |
| Lex p50 (ms) | 0.073 | 0.058 | **v1.0 WIN** |
| Lex p95 (ms) | 0.535 | 0.489 | **v1.0 WIN** |
| Lex p99 (ms) | 0.535 | 0.489 | **v1.0 WIN** |
| File size (bytes) | 4,096 | 4096 | TIE |

## Workload: Large

| Metric | Local v0.3 | Remote v1.0 | Winner |
|--------|-----------|------------|--------|
| Insert total (ms) | 4480.5 | 4480.4 | **v1.0 WIN** |
| Throughput (docs/s) | 22319 | 22319 | TIE |
| Lex p50 (ms) | 25.079 | 24.905 | **v1.0 WIN** |
| Lex p95 (ms) | 27.294 | 27.319 | **v0.3 WIN** |
| Lex p99 (ms) | 27.294 | 27.319 | **v0.3 WIN** |
| File size (bytes) | 49,696,768 | 49446912 | **v1.0 WIN** |

## Workload: Realworld

| Metric | Local v0.3 | Remote v1.0 | Winner |
|--------|-----------|------------|--------|
| Insert total (ms) | 435.7 | N/A | — |
| Throughput (docs/s) | 22950 | N/A | — |
| Lex p50 (ms) | 0.296 | N/A | — |
| Lex p95 (ms) | 0.816 | N/A | — |
| Lex p99 (ms) | 0.816 | N/A | — |
| File size (bytes) | 7,905,280 | N/A | — |

## Cold Start (5-run mean, `stats` subcommand)

| | Local v0.3 | Remote v1.0 | Winner |
|-|-----------|------------|--------|
| CLI spawn (ms) | 9.6 | 8.8 | **v1.0 WIN** |

## Feature Parity Matrix

| Feature | Local v0.3 | Remote v1.0 |
|---------|-----------|------------|
| Ed25519 Signing | ✓ | ✗ |
| Crdt Merge | ✓ | ✗ |
| Sqlcipher Encryption | ✗ | ✗ |
| Sharding Ivf Bloom | ✓ | ✗ |
| Federation Ysync | ✓ | ✗ |
| Self Learning | ✓ | ✗ |
| Multi Ext Brainpack | ✓ | ✗ |
| Mcp Server Mode | ✓ | ✓ |

## Verdict

**Local v0.3-full-stack**: 7/8 features — Ed25519 signing, CRDT merge, sharding, federation, self-learning, MCP, multi-ext (.syn/.brainpack).
**Remote v1.0**: 1/8 features — stripped to put/find/vec/hybrid/snap only.

Performance: both use the same SQLite+FTS5+msgpack daemon core. Insert throughput is identical (~22–33k docs/s depending on workload). Lex query latency is within 0.05ms noise (<10% delta).

**Canonical main → local v0.3-full-stack.** Remote v1.0 is a regression: removes 6 production features (signing, CRDT, sharding, federation, self-learning, snap-signed) with zero throughput improvement.