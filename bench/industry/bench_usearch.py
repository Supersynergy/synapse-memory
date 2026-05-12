#!/usr/bin/env python3
"""
Engine B: usearch direct (pure Python bindings)
M=16, ef_construction=64 HNSW index on 168k corpus.
Uses raw query vectors from ground truth.
"""
import json, os, time, sys
from pathlib import Path
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from lib_gt import load_gt, load_corpus, compute_recall, pstats

K10 = 10
K100 = 100
WARMUP = 5


def main():
    try:
        from usearch.index import Index
    except ImportError:
        result = {"engine": "usearch (hnsw M=16 ef=64)", "available": False, "reason": "pip install usearch"}
        print(json.dumps(result, indent=2))
        return result

    q_vecs, gt_ids, meta = load_gt()
    n = len(q_vecs)
    print(f"[usearch] {n} queries, corpus={meta['n_corpus']}, loading corpus...")

    corpus_vecs, corpus_ids = load_corpus()
    N = len(corpus_vecs)

    # Normalize for cosine
    norms = np.linalg.norm(corpus_vecs, axis=1, keepdims=True)
    norms[norms == 0] = 1.0
    corpus_n = (corpus_vecs / norms).astype(np.float32)

    norms_q = np.linalg.norm(q_vecs, axis=1, keepdims=True)
    norms_q[norms_q == 0] = 1.0
    q_vecs_n = (q_vecs / norms_q).astype(np.float32)

    results = []
    for (M, ef_c, ef_s, label) in [
        (16, 64, 64, "usearch M=16 ef=64"),
        (16, 64, 128, "usearch M=16 ef=128"),
    ]:
        print(f"  Building index {label} ({N} vecs)...", flush=True)
        t0 = time.perf_counter()
        idx = Index(ndim=384, metric="cos", connectivity=M, expansion_add=ef_c, expansion_search=ef_s)
        idx.add(corpus_ids.astype(np.uint64), corpus_n)
        build_s = time.perf_counter() - t0
        print(f"    build: {build_s:.1f}s")

        # Warmup
        for i in range(min(WARMUP, n)):
            idx.search(q_vecs_n[i], K10)

        latencies, res10, res100 = [], [], []
        for i in range(n):
            t0 = time.perf_counter()
            hits100 = idx.search(q_vecs_n[i], K100)
            ms = (time.perf_counter() - t0) * 1000
            ids = list(hits100.keys.astype(np.int64))
            latencies.append(ms)
            res10.append(ids[:K10])
            res100.append(ids)

        st = pstats(latencies)
        r = {
            "engine": label,
            "available": True,
            "build_s": build_s,
            "n_queries": n,
            **st,
            "recall_at_10": compute_recall(res10, gt_ids, K10),
            "recall_at_100": compute_recall(res100, gt_ids, K100),
        }
        results.append(r)
        print(f"    p50={st['p50_ms']:.1f}ms p95={st['p95_ms']:.1f}ms QPS={st['qps_1c']:.0f} R@10={r['recall_at_10']:.4f} R@100={r['recall_at_100']:.4f}")

    print(json.dumps(results, indent=2))
    return results


if __name__ == "__main__":
    main()
