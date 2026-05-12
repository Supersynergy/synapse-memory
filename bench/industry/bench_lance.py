#!/usr/bin/env python3
"""
Engine D: lance (IVF_PQ + HNSW)
Uses raw query vectors from ground truth.
Skips gracefully if lance not installed.
"""
import json, os, time, sys, tempfile, shutil
from pathlib import Path
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from lib_gt import load_gt, load_corpus, compute_recall, pstats

K10 = 10
K100 = 100
WARMUP = 5


def main():
    try:
        import lance
        import pyarrow as pa
    except ImportError as e:
        result = {"engine": "lance (IVF_PQ)", "available": False, "reason": f"import error: {e}"}
        print(json.dumps(result, indent=2))
        return result

    q_vecs, gt_ids, meta = load_gt()
    n = len(q_vecs)
    print(f"[lance] {n} queries, corpus={meta['n_corpus']}, loading corpus...")

    corpus_vecs, corpus_ids = load_corpus()
    N = len(corpus_vecs)

    norms = np.linalg.norm(corpus_vecs, axis=1, keepdims=True)
    norms[norms == 0] = 1.0
    corpus_n = (corpus_vecs / norms).astype(np.float32)

    norms_q = np.linalg.norm(q_vecs, axis=1, keepdims=True)
    norms_q[norms_q == 0] = 1.0
    q_vecs_n = (q_vecs / norms_q).astype(np.float32)

    tmpdir = tempfile.mkdtemp(prefix="lance_bench_")
    try:
        db_path = os.path.join(tmpdir, "bench.lance")
        print(f"  Writing lance dataset to {db_path}...", flush=True)
        t0 = time.perf_counter()

        table = pa.table({
            "id": pa.array(corpus_ids.tolist(), type=pa.int64()),
            "vector": pa.array(corpus_n.tolist(), type=pa.list_(pa.float32(), 384)),
        })
        ds = lance.write_dataset(table, db_path)
        write_s = time.perf_counter() - t0

        results = []
        for index_type, index_label in [("IVF_PQ", "lance IVF_PQ"), ("IVF_HNSW_SQ", "lance IVF_HNSW_SQ")]:
            print(f"  Building {index_label}...", flush=True)
            t0 = time.perf_counter()
            try:
                ds.create_index(
                    "vector",
                    index_type=index_type,
                    metric="cosine",
                    num_partitions=256,
                    num_sub_vectors=48,
                    replace=True,
                )
                build_s = time.perf_counter() - t0
            except Exception as e:
                results.append({"engine": index_label, "available": False, "reason": str(e)})
                continue
            print(f"    build: {build_s:.1f}s")

            # Warmup
            for i in range(min(WARMUP, n)):
                ds.to_table(nearest={"column": "vector", "q": q_vecs_n[i].tolist(), "k": K10}).to_pydict()

            latencies, res10, res100 = [], [], []
            for i in range(n):
                t0 = time.perf_counter()
                tbl = ds.to_table(nearest={"column": "vector", "q": q_vecs_n[i].tolist(), "k": K100})
                ms = (time.perf_counter() - t0) * 1000
                ids = tbl.to_pydict().get("id", [])
                latencies.append(ms)
                res10.append(ids[:K10])
                res100.append(ids)

            st = pstats(latencies)
            r = {
                "engine": index_label,
                "available": True,
                "build_s": build_s,
                "n_queries": n,
                **st,
                "recall_at_10": compute_recall(res10, gt_ids, K10),
                "recall_at_100": compute_recall(res100, gt_ids, K100),
            }
            results.append(r)
            print(f"    p50={st['p50_ms']:.1f}ms p95={st['p95_ms']:.1f}ms QPS={st['qps_1c']:.0f} R@10={r['recall_at_10']:.4f}")

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)

    print(json.dumps(results, indent=2))
    return results


if __name__ == "__main__":
    main()
