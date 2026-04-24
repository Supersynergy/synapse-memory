"""Generic real-world bench harness.

Loads a corpus (dir of text files, one per doc), embeds with a user-supplied
embedder, builds Synapse indices, runs a set of queries, and prints the
Synapse "Consumer Metrics" table.

Swap the embedder for any real model; the default is a tiny deterministic
hash embedder so the harness runs without network or GPU.

Usage:
    python harness.py --src ~/notes --queries queries.txt --dim 128
"""

from __future__ import annotations
import argparse
import math
import os
import random
import statistics
import sys
import time
from pathlib import Path
from typing import Callable, Iterable, List, Sequence, Tuple

try:
    import synapse  # noqa: F401
except ImportError:  # pragma: no cover
    print("synapse not installed. Run: maturin develop --release --features simsimd",
          file=sys.stderr)
    sys.exit(1)


# ---------------- Embedders -----------------------------------------------

class HashEmbedder:
    """Deterministic tiny embedder — no network, no GPU, for harness demos."""
    def __init__(self, dim: int = 128): self.dim = dim
    def _h(self, s: str) -> List[float]:
        h = hash(s) & 0xFFFFFFFFFFFFFFFF
        return [math.sin((h >> (i % 32)) * 0.0001 + i) for i in range(self.dim)]
    def embed_query(self, t: str) -> List[float]: return self._h(t)
    def embed_documents(self, texts: Sequence[str]) -> List[List[float]]:
        return [self._h(t) for t in texts]


# ---------------- Harness core --------------------------------------------

def load_corpus(src: Path, max_docs: int | None = None) -> List[Tuple[str, str]]:
    """Return list of (doc_id, text) from a directory of text files."""
    out: List[Tuple[str, str]] = []
    for p in sorted(src.rglob("*")):
        if not p.is_file(): continue
        if p.suffix.lower() not in {".md", ".txt", ".rst"}: continue
        try:
            out.append((str(p.relative_to(src)), p.read_text(encoding="utf-8", errors="ignore")))
        except Exception:
            continue
        if max_docs and len(out) >= max_docs: break
    return out


def build_indices(rows: List[Tuple[int, List[float]]]):
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    return ham, i8


def run_queries(
    embedder,
    ham, i8,
    queries: Sequence[str],
    k: int = 10,
    candidates: int = 80,
) -> List[float]:
    durations_us: List[float] = []
    for q in queries:
        vec = embedder.embed_query(q)
        t0 = time.perf_counter()
        if ham and i8 and i8.len() >= 1_000:
            _ = synapse.rerank(ham, i8, vec, k=k, candidates=candidates)
        else:
            _ = i8.search(vec, k=k)
        durations_us.append((time.perf_counter() - t0) * 1e6)
    return durations_us


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Synapse real-world bench harness")
    ap.add_argument("--src", type=Path, required=True, help="corpus dir (markdown/txt files)")
    ap.add_argument("--queries", type=Path, help="newline-separated queries file")
    ap.add_argument("--dim", type=int, default=128)
    ap.add_argument("--max-docs", type=int, default=None)
    ap.add_argument("--k", type=int, default=10)
    ap.add_argument("--candidates", type=int, default=80)
    args = ap.parse_args(argv)

    embedder = HashEmbedder(dim=args.dim)

    print(f"▸ loading corpus from {args.src} …")
    docs = load_corpus(args.src, args.max_docs)
    if not docs:
        print("no documents found", file=sys.stderr); return 1
    print(f"▸ loaded {len(docs):,} docs")

    t_embed = time.perf_counter()
    embs = embedder.embed_documents([t for _, t in docs])
    t_embed = time.perf_counter() - t_embed
    rows = [(i, v) for i, v in enumerate(embs)]

    t_build = time.perf_counter()
    ham, i8 = build_indices(rows)
    t_build = time.perf_counter() - t_build

    # Queries fallback: reuse 20 random doc-titles as queries.
    if args.queries and args.queries.exists():
        queries = [q.strip() for q in args.queries.read_text().splitlines() if q.strip()]
    else:
        random.seed(0)
        queries = [docs[random.randrange(len(docs))][0] for _ in range(20)]

    durs = run_queries(embedder, ham, i8, queries, k=args.k, candidates=args.candidates)
    durs_sorted = sorted(durs)
    def pct(p: float) -> float: return durs_sorted[min(len(durs_sorted) - 1, int(p * len(durs_sorted)))]

    print()
    print(f"▸ corpus embed time     {t_embed:>7.2f} s")
    print(f"▸ index build time      {t_build * 1000:>7.1f} ms")
    print(f"▸ queries run           {len(durs)}")
    print(f"▸ p50 latency           {statistics.median(durs):>7.0f} µs")
    print(f"▸ p95 latency           {pct(0.95):>7.0f} µs")
    print(f"▸ p99 latency           {pct(0.99):>7.0f} µs")
    print(f"▸ mean QPS              {1e6 / max(statistics.mean(durs), 1e-3):>7.0f}")
    print()
    print(f"consumer framing: «find anything across {len(docs):,} docs in {pct(0.95) / 1000:.1f} ms (95 %)»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
