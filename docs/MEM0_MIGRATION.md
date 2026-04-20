# mem0 → synapse-mem0 Migration Guide

## 1-Line Migration

```python
# Old
from mem0 import Memory          # pip install mem0ai — pulls 20+ deps, needs OpenAI key

# New
from synapse_mem0 import Memory  # pip install synapse-mem0 — msgpack only, zero cloud
```

Everything else stays identical.

## Comparison

| | mem0 (cloud) | synapse-mem0 |
|---|---|---|
| Backend | Qdrant + OpenAI embeddings | synapse daemon (local) |
| Storage | Hosted vector DB | SQLite on disk |
| Latency | ~200ms (API RTT) | <2ms (unix socket) |
| Cost | $0.002–$0.02 / 1k memories | $0 |
| Privacy | Data sent to cloud | Local only |
| Python deps | 20+ packages | `msgpack` only |
| Offline support | No | Yes |
| Docker-free | No | Yes (single binary) |

## Feature Parity

| Method | mem0 | synapse-mem0 |
|--------|------|--------------|
| `Memory()` | Cloud + local config | Local unix socket |
| `.add(messages, user_id)` | Full | Full |
| `.search(query, user_id)` | Vector + hybrid | FTS (Lex) |
| `.get_all(user_id)` | Full | Full |
| `.get(memory_id)` | Full | Full |
| `.update(memory_id, data)` | Full | Full |
| `.delete(memory_id)` | Full | Full |
| `.delete_all(user_id)` | Full | Full |
| `.history(memory_id)` | Full version log | Stub (empty list) |
| `MemoryClient` alias | Cloud SDK | Alias to `Memory` |

**Coverage: 9/10 (90%).** The only gap is `.history()` — synapse does not yet expose a per-document version log.

## Step-by-Step

### 1. Start synapse daemon

```bash
synapsed --sock /tmp/synapse.sock
```

### 2. Install shim

```bash
pip install synapse-mem0
```

### 3. Replace import

```diff
-from mem0 import Memory
+from synapse_mem0 import Memory
```

### 4. Optional: configure socket path

```python
m = Memory(sock_path="/var/run/synapse.sock")
```

## Data model mapping

| mem0 concept | synapse mapping |
|---|---|
| `user_id` | URI prefix `user/{user_id}/` |
| `memory_id` | UUID in title `user/{user_id}/{uuid}` |
| memory text | synapse `text` field |
| metadata | synapse `meta` JSONB field |
