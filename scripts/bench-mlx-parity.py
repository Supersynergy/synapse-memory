#!/usr/bin/env python3
"""Parity bench: self-converted MLX bf16 BGE vs fastembed CPU canonical.

For each of 50 paraphrase-style sentences, embed with both backends and
compute cosine similarity between the two 384-d vectors. Reports mean,
worst-case, and percentile breakdown. Target: mean >=0.99, worst >=0.985.

Also re-runs against upstream `mlx-community/bge-small-en-v1.5-bf16` for
A/B comparison (if cached) so the regression vs the broken upstream is
explicit in the output.
"""
from __future__ import annotations

import argparse
import os
import statistics
import sys
from typing import Iterable

import numpy as np

SENTENCES = [
    "The quick brown fox jumps over the lazy dog.",
    "A swift auburn fox leapt across the sleepy hound.",
    "Authentication uses JWT tokens signed with RS256.",
    "Login flow issues a JSON Web Token signed by RS256.",
    "MLX runs neural networks natively on Apple Silicon GPUs.",
    "Apple Silicon Metal acceleration powers MLX inference.",
    "Vector databases store high-dimensional embeddings for similarity search.",
    "Embedding stores enable nearest-neighbor lookup over dense vectors.",
    "PostgreSQL is a powerful open-source relational database.",
    "Postgres is an enterprise-grade open-source SQL database.",
    "Rust prevents data races at compile time via the borrow checker.",
    "The Rust borrow checker eliminates concurrent data races statically.",
    "Transformers use self-attention to model long-range dependencies.",
    "Self-attention layers in Transformers capture long-range token relations.",
    "Docker packages applications into portable container images.",
    "Containers built with Docker bundle apps for portable deployment.",
    "Kubernetes orchestrates containerized workloads across clusters.",
    "K8s schedules and manages container workloads on cluster nodes.",
    "Redis is an in-memory key-value data store used for caching.",
    "Redis serves as a fast in-memory cache and key-value database.",
    "GraphQL allows clients to request exactly the data they need.",
    "With GraphQL, clients fetch precisely the fields required.",
    "WebAssembly runs sandboxed binaries inside the browser.",
    "WASM executes portable sandboxed bytecode in browser runtimes.",
    "TypeScript adds static typing on top of JavaScript.",
    "TypeScript layers a static type system over JavaScript.",
    "SQLite is a serverless embedded SQL database engine.",
    "SQLite provides a file-based embedded relational engine.",
    "FTS5 is a full-text search extension for SQLite.",
    "SQLite ships an inverted-index full-text search module called FTS5.",
    "Cosine similarity measures the angle between two vectors.",
    "The cosine metric scores vector pairs by their angular distance.",
    "Quantization reduces model size with minor accuracy loss.",
    "Compressing model weights via quantization saves space at small cost.",
    "Adam is a popular adaptive learning-rate optimizer.",
    "The Adam optimizer adapts per-parameter learning rates during training.",
    "Async I/O scales servers by avoiding thread-per-connection.",
    "Asynchronous I/O lets servers handle many connections without per-thread overhead.",
    "BERT learns bidirectional context via masked language modeling.",
    "Masked LM objectives train BERT to use bidirectional context.",
    "Mean pooling averages token vectors for sentence embeddings.",
    "Sentence vectors can be obtained by averaging token-level embeddings.",
    "Cloudflare Workers run JavaScript at edge locations worldwide.",
    "JS runs at Cloudflare's global edge via Workers.",
    "OAuth2 delegates authorization without sharing credentials.",
    "OAuth2 lets apps act on a user's behalf without password sharing.",
    "Bloom filters test set membership with no false negatives.",
    "A Bloom filter answers membership queries with possible false positives only.",
    "Reinforcement learning trains agents via reward signals.",
    "RL agents learn behavior policies from environment rewards.",
]


def fastembed_vecs(texts: list[str]) -> np.ndarray:
    from fastembed import TextEmbedding

    model = TextEmbedding(model_name="BAAI/bge-small-en-v1.5")
    arr = np.array(list(model.embed(texts)), dtype=np.float32)
    norms = np.linalg.norm(arr, axis=1, keepdims=True).clip(min=1e-12)
    return arr / norms


def mlx_vecs(texts: list[str], model_path: str) -> np.ndarray:
    import mlx.core as mx
    from mlx_embeddings.utils import load

    model, tokenizer = load(model_path)
    enc = tokenizer.batch_encode_plus(
        texts, return_tensors="mlx", padding=True, truncation=True, max_length=512
    )
    ids, mask = enc["input_ids"], enc["attention_mask"]
    out = model(ids, attention_mask=mask)
    lhs = getattr(out, "last_hidden_state", None)
    if lhs is None:
        lhs = out[0] if isinstance(out, (tuple, list)) else out
    # BGE uses CLS pooling per its 1_Pooling/config.json (NOT mean pooling).
    # This was the actual source of the 0.91-0.94 drift.
    pooled = lhs[:, 0, :]
    norms = mx.maximum(mx.linalg.norm(pooled, axis=1, keepdims=True), mx.array(1e-12))
    pooled = pooled / norms
    mx.eval(pooled)
    return np.array(pooled.tolist(), dtype=np.float32)


def cos_per_row(a: np.ndarray, b: np.ndarray) -> np.ndarray:
    return (a * b).sum(axis=1)  # both already L2-normalized


def report(label: str, fe: np.ndarray, ml: np.ndarray) -> dict:
    cos = cos_per_row(fe, ml)
    d = {
        "label": label,
        "n": int(cos.shape[0]),
        "mean": float(cos.mean()),
        "median": float(np.median(cos)),
        "min": float(cos.min()),
        "p05": float(np.percentile(cos, 5)),
        "p10": float(np.percentile(cos, 10)),
        "max": float(cos.max()),
    }
    print(
        f"[{label:>30}] n={d['n']:3d}  mean={d['mean']:.4f}  median={d['median']:.4f}  "
        f"min={d['min']:.4f}  p05={d['p05']:.4f}  p10={d['p10']:.4f}  max={d['max']:.4f}"
    )
    return d


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--local",
        default="/Users/master/projects/synapse/models/bge-small-mlx-bf16",
        help="Path to self-converted MLX bf16 model dir",
    )
    ap.add_argument(
        "--upstream",
        default="mlx-community/bge-small-en-v1.5-bf16",
        help="Upstream HF id for A/B (skip with --no-upstream)",
    )
    ap.add_argument("--no-upstream", action="store_true")
    args = ap.parse_args()

    print(f"[bench] {len(SENTENCES)} sentences  fastembed (canonical) vs MLX bf16")
    fe = fastembed_vecs(SENTENCES)
    print(f"[fastembed] shape={fe.shape}  dtype={fe.dtype}")

    print(f"[mlx local] loading {args.local}")
    ml_local = mlx_vecs(SENTENCES, args.local)
    r_local = report("self-converted bf16", fe, ml_local)

    r_up = None
    if not args.no_upstream:
        try:
            print(f"[mlx upstream] loading {args.upstream}")
            ml_up = mlx_vecs(SENTENCES, args.upstream)
            r_up = report("upstream bf16 (broken)", fe, ml_up)
        except Exception as e:  # noqa: BLE001
            print(f"[skip upstream] {e}", file=sys.stderr)

    # Verdict
    pass_mean = r_local["mean"] >= 0.99
    pass_worst = r_local["min"] >= 0.985
    print()
    print(f"[verdict] mean>=0.99   : {'PASS' if pass_mean else 'FAIL'}  ({r_local['mean']:.4f})")
    print(f"[verdict] worst>=0.985 : {'PASS' if pass_worst else 'FAIL'}  ({r_local['min']:.4f})")
    if r_up is not None:
        print(
            f"[delta]   self-converted vs upstream mean: "
            f"{r_local['mean']:.4f} vs {r_up['mean']:.4f}  "
            f"(+{r_local['mean'] - r_up['mean']:.4f})"
        )

    return 0 if (pass_mean and pass_worst) else 2


if __name__ == "__main__":
    raise SystemExit(main())
