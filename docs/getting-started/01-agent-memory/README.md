# 01 — AI Agent Memory (Python)

Drop-in memory for any Python agent. Replaces Mem0 / ChromaDB.

## What you get

- Persistent, single-file store (`memory.db`)
- Session-scoped filtering via metadata
- Hybrid BM25 + vector recall in <10 ms

## Install

```bash
# Build from source (requires Rust + maturin)
cd /path/to/synapse
pip install maturin
maturin develop -p synapse-py --release

# Or: cargo build --release -p synapse-cli
# Then use via CLI (see main.py comments)
```

## Run

```bash
python main.py
```

## Expected output

```
Stored 4 memories
Top results for 'greetings':
  [1.00] user said hello (session=abc)
  [0.91] response: hi there! (session=abc)
  [0.74] session started for user alice (session=abc)

Session abc memories (recent):
  user asked about weather
  response: hi there!
  user said hello
  session started for user alice
```

## Key API

```python
from synapse_rs import Synapse

s = Synapse("./memory.db")

# Store with metadata
s.put("doc-id-1", "user said hello", metadata={"session": "abc", "role": "user"})

# Lexical search (fast, no embeddings needed)
results = s.search("greetings", k=3)
# → [(id, text, score), ...]

# Close when done
s.close()
```

## Next steps

- Add `sentence-transformers` for semantic search via `brain.put_with_embedding()`
- Wire into LangChain: see `integrations/langchain/`
- Scale to 10M+ docs: use `synapse-server` daemon mode
