# synapse-memory — Python SDK

Python client for [Synapse](https://github.com/Supersynergy/synapse) — single-file AI agent memory.

Unix socket + msgpack. Zero HTTP. Sub-ms calls when daemon is local.

## Install

```bash
pip install synapse-memory
# daemon: cargo install --git https://github.com/Supersynergy/synapse synapsed synapse-cli
synapsed -f ~/.synapse/brain.db &
```

## Usage

```python
from synapse_memory import Client

c = Client()  # connects to /tmp/synapse.sock
c.put("trailbase chosen over pocketbase", title="decision/backend")
for h in c.search("backend choice?", mode="hybrid", limit=5):
    print(h["score"], h["title"])
```

## Bank-scoped memory (Hindsight-compatible API)

```python
bank = c.bank("project/eventshub")
bank.retain("Phase 1 shipped 2026-04-18")
memories = bank.recall("what happened with phase 1?", limit=5)
```

## Batch ingest

```python
items = [{"text": "...", "title": "...", "meta": {...}} for doc in corpus]
ids = c.put_batch(items)  # 17k docs/s with cache, 338/s fresh
```

## CLI

```bash
syn ping
syn hybrid "query" 10
syn put "new memory"
syn put-batch < items.jsonl
syn bench
```

## Why synapse vs alternatives

- **vs mem0/Letta/Zep**: single file, no postgres, no server, Rust speed
- **vs Hindsight**: Ed25519 signed, CRDT merge, offline-first
- **vs raw sqlite-vec**: fastembed included, BM25+vec hybrid, signed brainpacks

See [Comparison docs](https://github.com/Supersynergy/synapse/blob/main/docs/COMPARISON.md).
