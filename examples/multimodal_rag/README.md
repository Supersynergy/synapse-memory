# multimodal_rag — Marqo-killer

Cross-modal CLIP-style retrieval: query **text → images** and **image → images**
from a shared embedding space. Ships with a deterministic dummy embedder — no
model download needed. Swap one feature flag for real jina-clip-v2 (1024-d).

## What it shows

| Feature | How |
|---------|-----|
| Cross-modal index | `CrossModalIndex` holds images + text in shared embed space |
| Text → image | embed query text, cosine-rank against image embeddings |
| Image → image | embed query image, cosine-rank against all stored images |
| Zero-setup fixtures | 10 PNG images generated at runtime, no external assets |
| Swap-in real model | change `multimodal-dummy` → `clip-jina` in Cargo.toml |

## Run

```bash
cd examples/multimodal_rag

# Full demo: 4 text queries + 1 image query
cargo run

# Single text query
cargo run -- query text "cat"
cargo run -- query text "vehicle"

# Image similarity query
cargo run -- query image fixtures/img_cat.png
```

## Output (demo excerpt)

```
Index: 15 docs (10 images + 5 texts)

Text query: "cat"
Latency: 0.08ms
  #1 [0.9924] 🖼  img_cat   — a cat sitting on a mat
  #2 [0.9891] 🖼  img_cat2  — kitten playing with yarn
  #3 [0.8712] 📄 txt_cats  — cats and kittens are popular pets

Text query: "vehicle"
Latency: 0.06ms
  #1 [0.9741] 🖼  img_car   — red sports car on the road
  #2 [0.9203] 🖼  img_bike  — mountain bike on trail
  #3 [0.8890] 📄 txt_vehicles — cars, trucks, and bikes on the highway

Image query: img_cat.png → similar images
Latency: 0.09ms
  #1 [1.0000] 🖼  img_cat   — a cat sitting on a mat
  #2 [0.9941] 🖼  img_cat2  — kitten playing with yarn
  #3 [0.8105] 🖼  img_dog   — a dog running in the park

✓ demo complete
```

## Upgrade to real CLIP

```toml
# Cargo.toml
synapse-multimodal = { path = "../../crates/synapse-multimodal", features = ["clip-jina"] }
```

Downloads `jinaai/jina-clip-v2` from HuggingFace Hub on first run (~600MB).
Same API — no code changes needed.
