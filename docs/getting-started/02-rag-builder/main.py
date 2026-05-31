"""
02-rag-builder — Hybrid BM25 + vector RAG with graph re-ranking.

Uses sentence-transformers for local embeddings.
Fallback: CLI-only mode (BM25 only, no embeddings).
"""

import argparse
import json
import os
import subprocess
import sys
import time

DOCS = [
    "Synapse is a single-file embedded vector database written in Rust.",
    "SimSIMD provides 71× peak speedup for dot-product via AVX-512.",
    "Synapse uses SimSIMD for SIMD-accelerated vector operations.",
    "HNSW index in pure Rust, no Python overhead, sub-millisecond recall.",
    "Rust async runtime Tokio powers the Synapse search daemon.",
    "Hybrid search fuses BM25 keyword scores with cosine similarity via RRF.",
    "HippoRAG-2 PPR graph re-ranking improves multi-hop question answering.",
    "Synapse is 970× faster than sqlite-vec at 1 million documents.",
    "Single binary deployment: no Docker, no cloud, just one file.",
    "Matryoshka embeddings allow truncation to 128 dims for 35× faster search.",
]


def embed_local(texts: list[str]) -> list[list[float]]:
    from sentence_transformers import SentenceTransformer
    model = SentenceTransformer("all-MiniLM-L6-v2")
    return model.encode(texts, normalize_embeddings=True).tolist()


def embed_openai(texts: list[str]) -> list[list[float]]:
    import openai
    client = openai.OpenAI()
    resp = client.embeddings.create(input=texts, model="text-embedding-3-small")
    return [d.embedding for d in resp.data]


def demo_cli(db: str, query: str) -> None:
    """CLI-only fallback (BM25 only)."""
    def synx(*args):
        r = subprocess.run(["synapse", "-f", db, *args], capture_output=True, text=True)
        return r.stdout.strip()

    synx("init")
    t0 = time.perf_counter()
    for i, text in enumerate(DOCS):
        synx("put", "--uri", f"doc-{i}", "--text", text)
    elapsed = time.perf_counter() - t0
    print(f"Ingested {len(DOCS)} docs in {elapsed:.2f}s (CLI, BM25 only)\n")

    print(f"Query: {query!r}")
    out = synx("find", query, "--limit", "5")
    for line in out.splitlines():
        if line.strip():
            print(" ", line)

    print("\nHybrid search:")
    out = synx("hybrid", query, "--limit", "5")
    for line in out.splitlines():
        if line.strip():
            print(" ", line)


def demo_python(db: str, query: str, embedder: str) -> None:
    from synapse_rs import Brain  # type: ignore

    embed_fn = embed_openai if embedder == "openai" else embed_local

    brain = Brain(db)
    t0 = time.perf_counter()
    embeddings = embed_fn(DOCS)
    for i, (text, emb) in enumerate(zip(DOCS, embeddings)):
        brain.put_with_embedding(text, emb, uri=f"doc-{i}", title=f"doc-{i}")
    elapsed = time.perf_counter() - t0
    print(f"Ingested {len(DOCS)} docs in {elapsed:.2f}s\n")

    q_emb = embed_fn([query])[0]

    print(f"Query: {query!r}")
    hits = brain.search_hybrid(query, q_emb, limit=5)
    for doc_id, text, score in hits:
        print(f"  [{score:.2f}] {text[:80]} (id={doc_id})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default="./rag.db")
    ap.add_argument("--query", default="fast vector search rust")
    ap.add_argument("--embedder", default="local", choices=["local", "openai"])
    args = ap.parse_args()

    if os.path.exists(args.db):
        os.remove(args.db)

    try:
        demo_python(args.db, args.query, args.embedder)
    except ImportError as e:
        print(f"[info] {e}, falling back to CLI mode")
        demo_cli(args.db, args.query)


if __name__ == "__main__":
    main()
