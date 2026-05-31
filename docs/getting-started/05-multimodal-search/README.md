# 05 — Multimodal Search (image + text)

Search across images and text in one unified store.  
Uses CLIP embeddings for images, MiniLM for text — both go into the same Synapse index.

## What you get

- Unified hybrid index: text + image in one `.db`
- Cross-modal queries: text query → find images, image query → find text
- Sub-10 ms recall on 10K items (M4 laptop)

## Requirements

```bash
pip install sentence-transformers Pillow torch
# Build synapse-py:
maturin develop -p synapse-py --release
```

## Run

```bash
# Ingest sample images + text, then query
python main.py --data-dir ../../sample-data/images-10

# Cross-modal: find images matching a text query
python main.py --query "a cat sitting on a chair"
```

## Expected output

```
Ingested 10 images + 20 text docs in 1.2s

Text query: "a cat sitting on a chair"
  [0.91] image: cat_chair.jpg (id=3)
  [0.84] image: living_room.jpg (id=7)
  [0.71] "The cat sat on the velvet chair" (id=12, text)

Image query: images-10/mountain.jpg → find similar
  [0.97] image: alpine_view.jpg (id=5)
  [0.88] "Snow-capped peaks at sunrise" (id=18, text)
```

## Key API

```python
brain = Brain("multimodal.db")

# Embed image with CLIP
img_emb = clip_encode_image("photo.jpg")          # → list[float] dim=512
brain.put_with_embedding("photo.jpg", img_emb, uri="img:photo.jpg")

# Embed text with same CLIP text encoder
txt_emb = clip_encode_text("a red sports car")
brain.put_with_embedding("a red sports car", txt_emb, uri="txt:caption-1")

# Cross-modal search: text → images
q_emb = clip_encode_text("cat on chair")
hits = brain.search_vec(q_emb, limit=10)
```

## Roadmap

- Video frame search (extract keyframes → CLIP → store)
- Audio search via Whisper transcription + embedding
- Server mode: `synapse-server --multimodal --port 8080`
