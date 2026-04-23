# synapse-langfuse

Langfuse tracing adapter for [synapse](https://github.com/Supersynergy/synapse) retrieval.

Interface spec: https://langfuse.com/docs/tracing (Langfuse Python SDK v3)

## Install

```bash
pip install synapse-langfuse
# With Langfuse tracing:
pip install "synapse-langfuse[langfuse]"
```

## Usage

```python
from langfuse import Langfuse
from synapsedb_langfuse import SynapseRetriever

lf = Langfuse()
retriever = SynapseRetriever(langfuse=lf, mode="Hybrid")

hits = retriever.search("capital of France", limit=5, trace_id="trace-001")
```

Each `search()` call emits a `synapse-retrieval` span with input, output, and latency.

## API

| Arg | Default | Description |
|-----|---------|-------------|
| `sock_path` | `/tmp/synapsedb.sock` | Unix socket path |
| `mode` | `Hybrid` | Search mode: Lex / Vec / Hybrid |
| `langfuse` | `None` | Langfuse client instance (optional) |
