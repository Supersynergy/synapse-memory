"""Photo-library CLIP search bench — e33.

Synthetic by default — generate N fake 512-dim CLIP-shaped vectors, query
with M random ones, report consumer metrics. Drop-in replace HashEmbedder
with a real CLIP embedder (open_clip, fashion-clip, Apple Photos NSVision)
to bench against actual photos.

Usage:
    python 33_photo_clip.py                     # synthetic 100k × 512
    python 33_photo_clip.py --n 1_000_000       # big-library stress
"""

from __future__ import annotations
import argparse
import math
import random
import statistics
import sys
import time

import synapse


def synthesize(n: int, dim: int, seed: int = 42) -> list[tuple[int, list[float]]]:
    rng = random.Random(seed)
    out: list[tuple[int, list[float]]] = []
    for i in range(n):
        v = [rng.gauss(0.0, 1.0) for _ in range(dim)]
        norm = math.sqrt(sum(x * x for x in v)) or 1.0
        v = [x / norm for x in v]
        out.append((i, v))
    return out


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=100_000)
    ap.add_argument("--dim", type=int, default=512)
    ap.add_argument("--queries", type=int, default=30)
    args = ap.parse_args(argv)

    print(f"▸ synthesizing {args.n:,} × {args.dim}-dim CLIP-shape vectors …")
    t0 = time.perf_counter()
    rows = synthesize(args.n, args.dim)
    t_gen = time.perf_counter() - t0

    t0 = time.perf_counter()
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    t_build = (time.perf_counter() - t0) * 1000

    rng = random.Random(99)
    queries = [rows[rng.randrange(args.n)][1] for _ in range(args.queries)]
    durs = []
    for q in queries:
        t = time.perf_counter()
        _ = synapse.rerank(ham, i8, q, k=10, candidates=80)
        durs.append((time.perf_counter() - t) * 1e6)
    ds = sorted(durs)
    def pct(p): return ds[min(len(ds) - 1, int(p * len(ds)))]

    print(f"▸ synth time             {t_gen:>7.2f} s")
    print(f"▸ build indices          {t_build:>7.1f} ms")
    print(f"▸ p50 / p95 / p99        {statistics.median(durs):>5.0f} / {pct(0.95):>5.0f} / {pct(0.99):>5.0f} µs")
    print()
    print(f"consumer framing: «'beach with dog' across {args.n:,} photos → {pct(0.95)/1000:.1f} ms»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
