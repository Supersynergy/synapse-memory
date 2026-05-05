#!/usr/bin/env python3
"""
Phase A fix: synapse-ultra via /vec_raw (stored query vectors, no re-embedding).
Phase B: ef_search sweep via usearch.
Phase C: binary cascade via ultra binary modes.

All recall computed vs brute-force GT (same basis as usearch/lance).
"""
import json, os, time, sys
import urllib.request
from pathlib import Path
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from lib_gt import load_gt, load_corpus, compute_recall, pstats

ULTRA_URL = os.environ.get("ULTRA_URL", "http://127.0.0.1:9478")
K10 = 10
K100 = 100
WARMUP = 20


def ultra_ping():
    try:
        with urllib.request.urlopen(f"{ULTRA_URL}/ping", timeout=2):
            return True
    except Exception:
        return False


def ultra_raw_search(vec: list, k: int, mode: str) -> tuple[list, float]:
    body = json.dumps({"vec": vec, "limit": k, "mode": mode}).encode()
    req = urllib.request.Request(
        f"{ULTRA_URL}/vec_raw",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            hits = json.loads(r.read())
        ms = (time.perf_counter() - t0) * 1000
        ids = [h.get("id") or h.get("doc_id") for h in hits]
        return ids, ms
    except Exception:
        return [], (time.perf_counter() - t0) * 1000


# ── Phase A: ultra /vec_raw modes ─────────────────────────────────────────────
def bench_ultra_raw(q_vecs_n: np.ndarray, gt_ids: list, mode: str) -> dict:
    label = f"ultra_raw ({mode})"
    if not ultra_ping():
        return {"engine": label, "available": False, "reason": f"ultra not at {ULTRA_URL}"}

    n = len(q_vecs_n)
    for i in range(min(WARMUP, n)):
        ultra_raw_search(q_vecs_n[i].tolist(), K10, mode)

    latencies, res10, res100 = [], [], []
    for i in range(n):
        ids100, ms = ultra_raw_search(q_vecs_n[i].tolist(), K100, mode)
        latencies.append(ms)
        res10.append(ids100[:K10])
        res100.append(ids100)

    st = pstats(latencies)
    return {
        "engine": label,
        "available": True,
        "mode": mode,
        "n_queries": n,
        **st,
        "recall_at_10": compute_recall(res10, gt_ids, K10),
        "recall_at_100": compute_recall(res100, gt_ids, K100),
        "note": "stored query vecs → /vec_raw (no re-embed). Fair comparison vs GT.",
    }


# ── Phase B: usearch ef_search sweep ──────────────────────────────────────────
def bench_usearch_sweep(q_vecs_n: np.ndarray, gt_ids: list,
                         corpus_n: np.ndarray, corpus_ids: np.ndarray) -> list[dict]:
    try:
        from usearch.index import Index
    except ImportError:
        return [{"engine": "usearch sweep", "available": False, "reason": "pip install usearch"}]

    results = []
    for M in [16, 32, 48]:
        for ef_s in [64, 128, 200, 400]:
            label = f"usearch M={M} ef={ef_s}"
            print(f"  Building {label}...", flush=True)
            t0 = time.perf_counter()
            idx = Index(ndim=384, metric="cos", connectivity=M,
                        expansion_add=max(ef_s, 64), expansion_search=ef_s)
            idx.add(corpus_ids.astype(np.uint64), corpus_n)
            build_s = time.perf_counter() - t0

            n = len(q_vecs_n)
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
            print(f"    p50={st['p50_ms']:.3f}ms QPS={st['qps_1c']:.0f} R@10={r['recall_at_10']:.4f}")
    return results


def main():
    q_vecs, gt_ids, meta = load_gt()
    corpus_vecs, corpus_ids = load_corpus()
    print(f"[iso-recall bench] {len(q_vecs)} queries, corpus={meta['n_corpus']}")

    # Normalize
    norms_q = np.linalg.norm(q_vecs, axis=1, keepdims=True)
    norms_q[norms_q == 0] = 1.0
    q_vecs_n = (q_vecs / norms_q).astype(np.float32)

    norms_c = np.linalg.norm(corpus_vecs, axis=1, keepdims=True)
    norms_c[norms_c == 0] = 1.0
    corpus_n = (corpus_vecs / norms_c).astype(np.float32)

    all_results = []

    # Phase A: ultra raw
    print("\n=== Phase A: ultra /vec_raw (stored vecs, no re-embed) ===")
    if ultra_ping():
        for mode in ["strict", "binary_first", "binary_only"]:
            print(f"  mode={mode}...", flush=True)
            r = bench_ultra_raw(q_vecs_n, gt_ids, mode)
            all_results.append(r)
            if r["available"]:
                print(f"    p50={r['p50_ms']:.3f}ms QPS={r['qps_1c']:.0f} R@10={r['recall_at_10']:.4f} R@100={r['recall_at_100']:.4f}")
    else:
        print("  SKIP: synapse-ultra not running at", ULTRA_URL)
        for mode in ["strict", "binary_first", "binary_only"]:
            all_results.append({"engine": f"ultra_raw ({mode})", "available": False, "reason": "server down"})

    # Phase B: usearch sweep
    print("\n=== Phase B: usearch ef_search sweep ===")
    sweep = bench_usearch_sweep(q_vecs_n, gt_ids, corpus_n, corpus_ids)
    all_results.extend(sweep)

    out = json.dumps(all_results, indent=2)
    print("\n=== RESULTS JSON ===")
    print(out)
    return all_results


if __name__ == "__main__":
    main()
