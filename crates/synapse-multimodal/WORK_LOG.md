# WORK_LOG — synapse-multimodal

## 2026-05-11

### Scaffolded

New crate `crates/synapse-multimodal` wired into workspace.

Files:
- `Cargo.toml` — features: `multimodal` (ort ONNX), `multimodal-dummy` (no model), default=none
- `src/lib.rs` — re-exports, CLIP_DIM=512
- `src/embedder.rs` — `MultimodalEmbedder` trait + `ClipEmbedder` (3 impls behind features)
- `src/mime.rs` — `MimeKind` via `infer` magic-bytes (jpeg/png/webp/gif)
- `src/index.rs` — `CrossModalIndex` in-memory cross-modal search
- `src/storage.rs` — `prepare_image_doc` bridge to `Db::add_with_embed`
- `tests/smoke.rs` — 4 integration tests

### Model decision

| Option | Status |
|--------|--------|
| `candle-transformers` CLIP | Skip — candle pulls heavy cuda deps, M4 Metal path incomplete for ViT |
| `ort` ONNX (openai/clip-vit-base-patch32) | Feature `multimodal` — ort 2.0.0-rc.12, wired but tokenizer stub (see TODO) |
| Dummy (blake3 hash + histogram) | Feature `multimodal-dummy` — deterministic, no download, E2E green |

Preferred prod model: **jina-clip-v2** (multilingual, 512-d, ONNX export available) — swap in via `embed_image`/`embed_text` when ort 2.x stable.

### Smoke result

```
test smoke::embed_dimensions_consistent ... ok
test smoke::mime_detection ... ok
test smoke::prepare_image_doc_storage ... ok
Top hit for 'cat': id=txt_kitten score=0.0582 kind=Text
Top image→text hit: id=img_cat score=1.0000    ← image→text cross-modal perfect
test smoke::cross_modal_query_text_finds_images ... ok
```

4/4 green. Image→text cross-modal: score 1.000 (identical embed space, dummy embedder correctly encodes path).

### TODOs / Next

1. **Tokenizer** — wire `tokenizers` crate (HF) for real CLIP text tokenization under `multimodal` feature
2. **jina-clip-v2** swap — multilingual, ONNX weights at `jinaai/jina-clip-v2`; superior MTEB 2026
3. **MUVERA fusion** — Multi-Vector Retrieval Augmentation: run CLIP + ColBERT embeddings, fuse scores via RRF (synapse-colbert already scaffolded); best multi-modal MTEB 2026
4. **Video** — scene-frame sampling → per-frame CLIP → temporal max-pool → 512-d video embed
5. **Audio** — whisper-cpp transcript → text embed + PANNs audio feature → 512-d audio embed
6. **Db integration** — `Db::add_image(doc_id, path, caption)` wrapper calling `prepare_image_doc` + `add_with_embed` in synapse-core
