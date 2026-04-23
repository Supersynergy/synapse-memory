# synapse-turbo v2

**1000x faster queries for your Synapse brain.db — Python daemon with 3-tier cache architecture.**

synapse-turbo is a companion tool that queries your existing `brain.db` at sub-millisecond latency by keeping the embedding model and vector matrix hot in memory. It uses the same `bge-small-en-v1.5` model and `sqlite-vec` schema as Synapse, so results are identical.

## Why

The Synapse Rust CLI loads the ONNX embedding model from disk on every invocation (~250ms). For interactive use (agent hooks, MCP tools, IDE integrations), that latency adds up. synapse-turbo solves this by running as a persistent daemon with:

| Tier | Strategy | Latency | When |
|------|----------|---------|------|
| T1 | Pre-computed results dict | 0.0003 ms | Repeated query |
| T2 | NumPy NEON brute-force kNN | 0.05 ms | Cached embedding, new query |
| T3 | fastembed ONNX (8 threads) | 3.2 ms | Never-seen query |

## Benchmarks (M4 Max, 3256 docs)

```
T1 dict[hash]              min=0.0000ms  p50=0.0001ms
T2 numpy cosine top-5      min=0.0218ms  p50=0.0250ms
sqlite-vec top-5           min=0.2126ms  p50=0.2350ms
FTS5 search                min=0.0486ms  p50=0.0550ms
fastembed ONNX 8t          min=3.2191ms  p50=3.4500ms

Daemon throughput: 5,921 queries/sec
```

NumPy brute-force beats sqlite-vec by 4.7x at this scale because ARM NEON SIMD matrix multiply has lower overhead than the ANN index for <50k documents.

## Requirements

```bash
pip install fastembed sqlite-vec numpy
# Optional (faster):
pip install orjson uvloop
```

Python 3.12+ recommended. Requires an existing `~/.synapse/brain.db` created by the Synapse CLI.

## Usage

### CLI (one-shot)

```bash
# FTS5 keyword search
python synapse_turbo.py find "database migration"

# Vector similarity search
python synapse_turbo.py vec "how to deploy to production"

# Hybrid search (FTS5 + vector + RRF fusion)
python synapse_turbo.py hybrid "rust web framework" --limit 5

# Self-benchmark
python synapse_turbo.py bench

# Cache stats
python synapse_turbo.py stats
```

### Daemon (persistent, recommended)

```bash
# Start daemon (port 9477)
python synapse_turbo.py daemon

# Query via daemon
python synapse_turbo.py q "rust web framework"

# Or via curl
curl "http://127.0.0.1:9477/hybrid?q=rust+web+framework&limit=5"
```

### HTTP API

The daemon exposes three endpoints:

```
GET /find?q=<query>&limit=<n>    # FTS5 keyword search
GET /vec?q=<query>&limit=<n>     # Vector similarity
GET /hybrid?q=<query>&limit=<n>  # Hybrid (recommended)
```

Response:
```json
{
  "mode": "hybrid",
  "query": "rust web framework",
  "elapsed_ms": 0.051,
  "count": 5,
  "results": [
    {"id": 42, "score": 0.032, "title": "Axum vs Actix", "text": "..."}
  ]
}
```

## macOS auto-start (launchd)

The plist is pre-configured for the default setup. Install:

```bash
# Copy to LaunchAgents
cp com.synapse.turbo.plist ~/Library/LaunchAgents/

# Load
launchctl load ~/Library/LaunchAgents/com.synapse.turbo.plist

# Verify
curl -s "http://127.0.0.1:9477/hybrid?q=test&limit=1" | python -m json.tool
```

## Architecture

```
Query arrives
    |
    v
[T1] hash lookup in pre-computed dict ──hit──> return (0.0003ms)
    |miss
    v
[T2] check embedding cache (in-memory dict > SQLite)
    |hit
    v
    NumPy matmul cosine (NEON SIMD) ──> RRF with FTS5 ──> return (0.05ms)
    |miss
    v
[T3] fastembed ONNX Runtime (8 threads, bge-small-en-v1.5)
    |
    v
    cache embedding ──> T2 search ──> cache results ──> return (3.2ms)
```

## Configuration

Edit the constants at the top of `synapse_turbo.py`:

| Constant | Default | Description |
|----------|---------|-------------|
| `BRAIN_DB` | `~/.synapse/brain.db` | Path to Synapse brain database |
| `CACHE_DB` | `~/.synapse/emb_cache.db` | Embedding cache location |
| `EMBED_MODEL` | `BAAI/bge-small-en-v1.5` | Must match Synapse's model |
| `EMBED_DIM` | `384` | Embedding dimensions |
| `DAEMON_PORT` | `9477` | HTTP daemon port |
| `ONNX_THREADS` | `8` | ONNX Runtime threads (tune for your CPU) |

## Thread tuning

The default 8 threads is optimal for Apple M4 Max. For other CPUs:

| CPU | Recommended threads |
|-----|-------------------|
| M1/M2 | 4 |
| M3/M4 | 6-8 |
| M4 Max/Ultra | 8 |
| Intel i7/i9 | 4-6 |
| AMD Ryzen 7/9 | 6-8 |

Run `synapse_turbo.py bench` with different `ONNX_THREADS` values to find your optimum.
