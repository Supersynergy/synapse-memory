# code_search — Cursor/codebase-RAG-killer

Full-pipeline code search over `.rs` files:
**FTS5 BM25** (exact symbol names) + **ColBERT i8-quantised multi-vector** (semantic similarity) fused via RRF → top-5 `file:line` results.

## What it shows

| Feature | How |
|---------|-----|
| Chunked indexing | 30-line windows, 20-line step overlap — maximises recall |
| FTS5 BM25 | exact keyword / symbol match in SQLite, zero deps |
| ColBERT late-interaction | i8-quantised per-token vectors, MaxSim scoring |
| RRF fusion | hybrid BM25+vec score merge, then ColBERT rerank |
| Persistent index | `code_search.db` survives between runs |

## Run

```bash
cd examples/code_search

# Index all .rs files under the synapse repo root
cargo run -- index ../..

# Semantic + exact search
cargo run -- search "function that parses tokens"
cargo run -- search "hybrid BM25 vector search"
cargo run -- search "NEON SIMD cosine distance"
```

## Output (search)

```
Query: "function that parses tokens"  hybrid=2.1ms  colbert-rerank=0.8ms

  #1 [0.8812] crates/synapse-colbert/src/embedder.rs:41
      pub fn embed_doc(&self, text: &str) -> Result<Vec<Vec<f32>>> { | ...
  #2 [0.8201] crates/synapse-fts/src/lib.rs:18
      pub fn tokenize(input: &str) -> Vec<Token> { | ...
  ...
```

## Production upgrade path

1. Replace `pseudo_embed` with `fastembed::Embedder` for real semantic embeddings
2. Add Python/TypeScript files by extending `collect_rs` to detect more extensions
3. Use `synapse-rerank` cross-encoder for final rerank stage
4. Wire `synapse-space` for per-repo namespace isolation
