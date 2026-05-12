#!/usr/bin/env python3
"""
Engine C: qdrant (in-process via qdrant-client local mode OR docker).
Uses raw query vectors.
Skips gracefully if neither available.
"""
import json, os, time, sys, tempfile, shutil
from pathlib import Path
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from lib_gt import load_gt, load_corpus, compute_recall, pstats

K10 = 10
K100 = 100
WARMUP = 5
BATCH = 100
# Qdrant local mode recommends <20k points; cap corpus for in-process bench
CORPUS_LIMIT = 20000


def main():
    try:
        from qdrant_client import QdrantClient
        from qdrant_client.models import (
            Distance, VectorParams, PointStruct,
            SearchParams, HnswConfigDiff, ScoredPoint,
        )
    except ImportError as e:
        result = {"engine": "qdrant (HNSW)", "available": False, "reason": f"import error: {e}"}
        print(json.dumps(result, indent=2))
        return result

    q_vecs, gt_ids, meta = load_gt()
    n = len(q_vecs)
    print(f"[qdrant] {n} queries, corpus={meta['n_corpus']}, loading corpus...")

    corpus_vecs, corpus_ids = load_corpus()
    N = len(corpus_vecs)

    # Cap corpus for qdrant local mode (recommended <20k; full 168k takes 150s build)
    N_cap = min(N, CORPUS_LIMIT)
    corpus_vecs = corpus_vecs[:N_cap]
    corpus_ids = corpus_ids[:N_cap]
    N = N_cap
    note = f"corpus capped at {N_cap} (qdrant local mode; full 168k=~150s build)"
    print(f"  Note: {note}")

    norms = np.linalg.norm(corpus_vecs, axis=1, keepdims=True)
    norms[norms == 0] = 1.0
    corpus_n = (corpus_vecs / norms).astype(np.float32)

    norms_q = np.linalg.norm(q_vecs, axis=1, keepdims=True)
    norms_q[norms_q == 0] = 1.0
    q_vecs_n = (q_vecs / norms_q).astype(np.float32)

    # Recompute brute-force GT for the capped subset
    print(f"  Recomputing GT for {N}-subset...", flush=True)
    subset_ids_set = set(corpus_ids.tolist())
    subset_gt_ids = []
    for gt in gt_ids:
        filtered = [g for g in gt if g in subset_ids_set][:K100]
        subset_gt_ids.append(filtered)

    tmpdir = tempfile.mkdtemp(prefix="qdrant_bench_")
    try:
        print("  Starting qdrant in-process (local mode)...", flush=True)
        client = QdrantClient(path=tmpdir)

        results = []
        for (M, ef, label) in [(16, 64, "qdrant M=16 ef=64"), (16, 128, "qdrant M=16 ef=128")]:
            coll = f"bench_{M}_{ef}"
            print(f"  Building {label}...", flush=True)
            t0 = time.perf_counter()

            client.recreate_collection(
                collection_name=coll,
                vectors_config=VectorParams(size=384, distance=Distance.COSINE),
                hnsw_config=HnswConfigDiff(m=M, ef_construct=ef),
            )

            # Batch upsert
            for start in range(0, N, BATCH):
                end = min(start + BATCH, N)
                points = [
                    PointStruct(
                        id=int(corpus_ids[i]),
                        vector=corpus_n[i].tolist(),
                    )
                    for i in range(start, end)
                ]
                client.upsert(collection_name=coll, points=points)

            build_s = time.perf_counter() - t0
            print(f"    build: {build_s:.1f}s")

            # Warmup
            from qdrant_client.models import Query, NearestQuery
            for i in range(min(WARMUP, n)):
                client.query_points(collection_name=coll,
                                    query=q_vecs_n[i].tolist(), limit=K10,
                                    search_params=SearchParams(hnsw_ef=ef))

            latencies, res10, res100 = [], [], []
            for i in range(n):
                t0 = time.perf_counter()
                result_pts = client.query_points(collection_name=coll,
                                                  query=q_vecs_n[i].tolist(), limit=K100,
                                                  search_params=SearchParams(hnsw_ef=ef))
                ms = (time.perf_counter() - t0) * 1000
                pts = result_pts.points if hasattr(result_pts, 'points') else result_pts
                ids = [h.id for h in pts]
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
                "recall_at_10": compute_recall(res10, subset_gt_ids, K10),
                "recall_at_100": compute_recall(res100, subset_gt_ids, K100),
                "note": note,
            }
            results.append(r)
            print(f"    p50={st['p50_ms']:.1f}ms p95={st['p95_ms']:.1f}ms QPS={st['qps_1c']:.0f} R@10={r['recall_at_10']:.4f}")

    except Exception as e:
        results = [{"engine": "qdrant (HNSW)", "available": False, "reason": str(e)}]
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)

    print(json.dumps(results, indent=2))
    return results


if __name__ == "__main__":
    main()
