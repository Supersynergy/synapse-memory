"""
05-multimodal-search — CLIP image + MiniLM text in one Synapse index.

Requires: pip install sentence-transformers Pillow torch
          maturin develop -p synapse-py --release
"""

import argparse
import os
import sys
from pathlib import Path

SAMPLE_CAPTIONS = [
    "A cat sitting on a velvet chair in the living room.",
    "Snow-capped mountain peaks at golden sunrise.",
    "Red sports car racing on a wet track at night.",
    "Children playing football on a green park lawn.",
    "Close-up of a steaming cup of black coffee.",
    "Aerial view of a coastal city at dusk.",
    "A dog chasing a frisbee on the beach.",
    "Abstract oil painting with bold blue strokes.",
    "Laptop and coffee on a wooden desk, cozy office.",
    "Fresh vegetables arranged on a farmers market stall.",
    "Neon-lit Tokyo street at midnight, reflections on wet asphalt.",
    "Old library with floor-to-ceiling bookshelves.",
    "A whale breaching near a small sailing boat.",
    "Portrait of a smiling person in natural light.",
    "Crystal clear mountain lake reflecting pine trees.",
    "Time-lapse of traffic lights and car trails at night.",
    "A bee landing on a yellow sunflower.",
    "Minimalist white kitchen with morning light.",
    "Group of friends laughing around a campfire.",
    "Aurora borealis over a frozen lake.",
]


def load_clip():
    from sentence_transformers import SentenceTransformer
    return SentenceTransformer("clip-ViT-B-32")


def load_text_model():
    from sentence_transformers import SentenceTransformer
    return SentenceTransformer("all-MiniLM-L6-v2")


def encode_image(model, path: str) -> list[float]:
    from PIL import Image
    img = Image.open(path).convert("RGB")
    return model.encode(img, normalize_embeddings=True).tolist()


def encode_text(model, text: str) -> list[float]:
    return model.encode(text, normalize_embeddings=True).tolist()


def demo_cli_only(data_dir: str | None, query: str) -> None:
    """BM25 fallback when Python bindings unavailable."""
    import subprocess, time

    db = "./multimodal.db"
    if os.path.exists(db):
        os.remove(db)

    def synx(*args):
        r = subprocess.run(["synapse", "-f", db, *args], capture_output=True, text=True)
        return r.stdout.strip()

    synx("init")
    t0 = time.perf_counter()
    for i, caption in enumerate(SAMPLE_CAPTIONS):
        synx("put", "--uri", f"txt:{i}", "--text", caption)

    if data_dir and os.path.isdir(data_dir):
        for img_path in Path(data_dir).glob("*.jpg"):
            synx("put", "--uri", f"img:{img_path.name}", "--text", f"[image] {img_path.stem}")

    elapsed = time.perf_counter() - t0
    print(f"Ingested {len(SAMPLE_CAPTIONS)} captions in {elapsed:.2f}s (CLI/BM25 mode)\n")

    print(f"Query: {query!r}")
    out = synx("hybrid", query, "--limit", "5")
    for line in out.splitlines():
        if line.strip():
            print(" ", line)


def demo_multimodal(data_dir: str | None, query: str) -> None:
    from synapse_rs import Brain  # type: ignore
    import time

    db = "./multimodal.db"
    if os.path.exists(db):
        os.remove(db)

    brain = Brain(db)

    print("Loading CLIP model...")
    clip = load_clip()

    t0 = time.perf_counter()

    # Ingest text captions
    for i, caption in enumerate(SAMPLE_CAPTIONS):
        emb = encode_text(clip, caption)
        brain.put_with_embedding(caption, emb, uri=f"txt:{i}", title=f"caption-{i}")

    img_count = 0
    if data_dir and os.path.isdir(data_dir):
        for img_path in Path(data_dir).glob("*.jpg"):
            emb = encode_image(clip, str(img_path))
            brain.put_with_embedding(f"[image] {img_path.stem}", emb,
                                     uri=f"img:{img_path.name}", title=img_path.name)
            img_count += 1

    elapsed = time.perf_counter() - t0
    print(f"Ingested {len(SAMPLE_CAPTIONS)} captions + {img_count} images in {elapsed:.2f}s\n")

    # Text → cross-modal search
    print(f"Query: {query!r}")
    q_emb = encode_text(clip, query)
    hits = brain.search_vec(q_emb, limit=5)
    for doc_id, text, score in hits:
        kind = "image" if text.startswith("[image]") else "text"
        print(f"  [{score:.2f}] [{kind}] {text[:70]} (id={doc_id})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data-dir", default=None,
                    help="Directory with .jpg files (optional)")
    ap.add_argument("--query", default="a cat sitting on a chair")
    args = ap.parse_args()

    try:
        demo_multimodal(args.data_dir, args.query)
    except ImportError as e:
        print(f"[info] {e} — falling back to CLI/BM25 mode")
        demo_cli_only(args.data_dir, args.query)


if __name__ == "__main__":
    main()
