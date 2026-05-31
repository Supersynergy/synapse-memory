# 02 — RAG Builder (Pinecone-killer)

Build a retrieval-augmented generation pipeline in ~50 lines.  
Single file, no cloud, 970× faster than sqlite-vec at 1M docs.

## What you get

- Ingest arbitrary text chunks with embeddings
- Hybrid BM25 + vector retrieval
- Graph-enhanced re-ranking (HippoRAG-2 PPR)
- JSON context bundle for your LLM prompt

## Install

```bash
pip install sentence-transformers  # or use OpenAI embeddings
# build synapse-py:
maturin develop -p synapse-py --release
# OR just use the CLI binary
cargo build --release -p synapse-cli
```

## Run

```bash
python main.py
# With OpenAI embeddings:
OPENAI_API_KEY=sk-... python main.py --embedder openai
```

## Expected output

```
Ingested 10 docs in 0.31 s
Query: "fast vector search rust"
  [0.94] Synapse uses SimSIMD for SIMD-accelerated vec ops (doc-3)
  [0.87] HNSW index in pure Rust, no Python overhead (doc-7)
  [0.81] Rust async runtime Tokio powers the search daemon (doc-5)

Graph-enhanced (PPR) top result:
  SimSIMD provides 71× peak speedup via AVX-512 (doc-4)
```

## Migration from Pinecone

```bash
# Export from Pinecone → JSONL, then:
synapse put-batch --file pinecone_export.jsonl -f rag.db
# Done. synapse hybrid "your query" -f rag.db
```

## Key API

```python
brain = Brain("rag.db")

# Insert with pre-computed embedding
brain.put_with_embedding(text, embedding=embed(text))

# Hybrid search: BM25 + cosine via RRF fusion
hits = brain.search_hybrid("rust vector", embed("rust vector"), limit=10)

# Graph re-rank via HippoRAG-2 PPR
# CLI: synapse graph ppr '{"<id>":1.0}' -f rag.db
```
