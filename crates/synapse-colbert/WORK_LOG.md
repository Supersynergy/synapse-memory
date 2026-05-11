# ColBERT-v2 Scaffold — WORK_LOG

## Was gebaut

### Crate: `crates/synapse-colbert`

**Files:**
- `Cargo.toml` — feature-gate `colbert` (default off), deps: rusqlite/zstd/serde_json/anyhow
- `src/lib.rs` — pub re-exports
- `src/embedder.rs` — `ColbertEmbedder { embed_doc, embed_query }` → `Vec<Vec<f32>>` (128-dim, L2-norm)
- `src/kernel.rs` — `max_sim(query_vecs, doc_vecs) -> f32` unroll-4 dot
- `src/store.rs` — `ColbertStore { add_colbert, embed_and_add, colbert_rerank }`

### Workspace
`Cargo.toml` — `"crates/synapse-colbert"` member eingefügt

---

## API

```rust
// Embed
let emb = ColbertEmbedder::default();
let doc_vecs: Vec<Vec<f32>> = emb.embed_doc("text")?;  // N × 128
let q_vecs:   Vec<Vec<f32>> = emb.embed_query("q")?;   // M × 128

// Kernel
let score: f32 = max_sim(&q_vecs, &doc_vecs);

// Storage + rerank
let store = ColbertStore::new(&conn)?;
store.embed_and_add(doc_id, "text")?;
let ranked: Vec<(i64, f32)> = store.colbert_rerank("query", &candidate_ids)?;
```

**Storage:** SQLite table `colbert_vecs(doc_id PK, vecs BLOB)` — zstd(json) per doc.

---

## Smoke Test

10 docs, query "ColBERT late interaction reranking":
- 5/5 tests green
- ColBERT-related docs (id 2, 9) in top-3 ✓
- Scores descending ✓

---

## Next (für prod)

1. **Model-Download** — jina-colbert-v2 via candle-core (HuggingFace Hub). `ColbertEmbedder::from_model(path)`. Feature-gate `colbert` aktivieren.
2. **Candle Metal Kernel** — NEON/Metal matmul für das MaxSim inner loop (statt scalar unroll). ~10× auf M4 Max erwartet.
3. **Batched add** — `add_colbert_batch` für Bulk-Ingest (jetzt 1 SQL/doc).
4. **ANN-Integration** — ColBERT rerank als zweite Stufe nach HNSW-ANN in synapse-ultra/synapse-ann.
5. **Quantisierung** — int8 token vecs (synapse-quant anbinden) → 4× Storage-Reduktion.

---

## Hebel-Summary

- **Zero-friction foundation**: plug-in reranker ohne Model-Dependency — smoke läuft heute, candle swap morgen
- **Compounding**: MaxSim kernel → später Metal-SIMD ersetzen ohne API-Änderung (open/closed)
- **Optionality**: feature-gate `colbert` off by default → kein compile-overhead für Nutzer die kein ColBERT brauchen
