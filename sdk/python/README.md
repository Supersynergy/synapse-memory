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

`bank.recall()` uses scoped fusion: indexed scope-first candidate fetch, one
daemon `BatchSearch` fallback, metadata scope filtering, and a small query-term
rerank. That keeps unrelated memories out of agent context without requiring a
separate vector database.

## AgentDB: public agent memory database API

```python
agent = c.agent("coder", project="my-app")

agent.observe(
    "Use scoped fusion for recall; it avoids leaking unrelated memories.",
    title="decision/recall-routing",
    kind="decision",
    tags=["recall", "architecture"],
    source_uri="file:///my-app/docs/ADR.md",
)

pack = agent.context_pack("how should recall route?", token_budget=800)
print(pack["context"])
agent.feedback("how should recall route?", [h["id"] for h in pack["index"][:2]], "accepted")
```

`AgentDB` is the public agent-facing layer:

- `observe()` / `remember()` stores typed, scoped memories with freshness metadata.
- `search_index()` returns compact first-pass hits for broad recall.
- `get_observations()` hydrates only the full memories an agent actually needs.
- `timeline()` returns recent scoped memories.
- `context_pack()` builds a token-budgeted XML context block.
- `feedback()` logs accepted/rejected recall outcomes for learned reranking.

This mirrors the best progressive-disclosure pattern from agent-memory tools,
but keeps Synapse's local single-file hot path: no Chroma, no Postgres, no HTTP
service in front of the daemon.

## Batch search

```python
results = c.batch_search([
    "backend choice?",
    {"query": "phase shipped", "mode": "lex", "limit": 3},
])
```

Use this for prompt hooks, evaluation loops, and context builders where fork or
socket overhead would otherwise dominate many tiny recall calls.

## Batch ingest

```python
items = [{"text": "...", "title": "...", "meta": {...}} for doc in corpus]
ids = c.put_batch(items)  # 17k docs/s with cache, 338/s fresh
```

## CLI

```bash
synx ping
synx hybrid "query" 10
synx put "new memory"
synx put-batch < items.jsonl
synx agent-observe coder "Phase 1 shipped"
synx agent-search coder "phase shipped"
synx agent-context coder "what changed?" 800
synx bench
```

## Why synapse vs alternatives

- **vs mem0/Letta/Zep**: single file, no postgres, no server, Rust speed
- **vs Hindsight**: Ed25519 signed, CRDT merge, offline-first
- **vs raw sqlite-vec**: fastembed included, BM25+vec hybrid, signed brainpacks

See [Comparison docs](https://github.com/Supersynergy/synapse/blob/main/docs/COMPARISON.md).
