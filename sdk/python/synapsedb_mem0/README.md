# synapse-mem0

Drop-in replacement for the `mem0` Python SDK, backed by [synapse](https://github.com/supersynergy/synapse).

## Install

```bash
pip install synapse-mem0
```

## Migrate in 1 line

```python
# Before
from mem0 import Memory

# After — nothing else changes
from synapsedb_mem0 import Memory
```

## Quickstart

```python
from synapsedb_mem0 import Memory

m = Memory()                          # connects to /tmp/synapsedb.sock

# Add memories
m.add("Alice loves hiking", user_id="alice")
m.add([{"role": "user", "content": "I prefer Python"}], user_id="alice")

# Search
results = m.search("hiking", user_id="alice")
for r in results["results"]:
    print(r["id"], r["memory"])

# Get all
all_mem = m.get_all(user_id="alice")

# Update
m.update(memory_id, "Alice loves trail running", user_id="alice")

# Delete
m.delete(memory_id, user_id="alice")
m.delete_all(user_id="alice")

# History (stub — returns empty list)
m.history(memory_id)
```

## Why synapse-mem0?

| Feature | mem0 | synapse-mem0 |
|---------|------|--------------|
| Backend | Qdrant + OpenAI + cloud | synapse daemon (local unix socket) |
| Cloud required | Yes (MemoryClient) | No — 100% local |
| Python deps | 20+ | `msgpack` only |
| Storage | Hosted vector DB | Single SQLite file per user |
| Speed | ~200ms API RTT | <2ms unix socket RTT |
| Cost | $0.002–$0.02 / 1k mems | $0 |
| Data privacy | Sent to cloud | Stays on disk |

## API Coverage

| mem0 method | Supported |
|-------------|-----------|
| `Memory()` | Yes |
| `.add(messages, user_id)` | Yes |
| `.search(query, user_id)` | Yes |
| `.get_all(user_id)` | Yes |
| `.get(memory_id)` | Yes |
| `.update(memory_id, data)` | Yes |
| `.delete(memory_id)` | Yes |
| `.delete_all(user_id)` | Yes |
| `.history(memory_id)` | Stub (returns `[]`) |
| `MemoryClient` alias | Yes |

**Coverage: 9/10 methods (90%). `history` returns stub — synapse has no version log.**

## Custom socket path

```python
m = Memory(sock_path="/var/run/synapse.sock")
```

## License

MIT
