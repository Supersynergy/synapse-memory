#!/usr/bin/env python3
"""
Phase A fix: synapse-ultra via /vec_raw (stored query vectors, no re-embedding).
Phase B: ef_search sweep via usearch.
Phase C: binary cascade via ultra binary modes.
Phase D: /vec_raw_batch — amortizes HTTP overhead, persistent connection.

All recall computed vs brute-force GT (same basis as usearch/lance).
"""
import json, os, time, sys
import http.client
import urllib.request
from pathlib import Path
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from lib_gt import load_gt, load_corpus, compute_recall, pstats

ULTRA_URL = os.environ.get("ULTRA_URL", "http://127.0.0.1:9478")
K10 = 10
K100 = 100
WARMUP = 20
# Queries per HTTP POST for batch mode — amortizes ~0.5ms TCP+parse overhead
BATCH_SIZE = int(os.environ.get("ULTRA_BATCH_SIZE", "32"))

_host, _port = None, None
def _parse_host_port():
    global _host, _port
    url = ULTRA_URL.removeprefix("http://").removeprefix("https://")
    if ":" in url:
        h, p = url.rsplit(":", 1)
        _host, _port = h, int(p)
    else:
        _host, _port = url, 80

_parse_host_port()


def _make_conn() -> http.client.HTTPConnection:
    conn = http.client.HTTPConnection(_host, _port, timeout=10)
    return conn


def ultra_ping():
    try:
        with urllib.request.urlopen(f"{ULTRA_URL}/ping", timeout=2):
            return True
    except Exception:
        return False


def ultra_raw_search(vec: list, k: int, mode: str, conn: http.client.HTTPConnection | None = None) -> tuple[list, float]:
    body = json.dumps({"vec": vec, "limit": k, "mode": mode}).encode()
    t0 = time.perf_counter()
    try:
        if conn is None:
            req = urllib.request.Request(
                f"{ULTRA_URL}/vec_raw",
                data=body,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req, timeout=10) as r:
                hits = json.loads(r.read())
        else:
            conn.request("POST", "/vec_raw", body, {"Content-Type": "application/json"})
            resp = conn.getresponse()
            hits = json.loads(resp.read())
        ms = (time.perf_counter() - t0) * 1000
        ids = [h.get("id") or h.get("doc_id") for h in hits]
        return ids, ms
    except Exception:
        return [], (time.perf_counter() - t0) * 1000


def ultra_raw_batch(vecs: list, k: int, mode: str, conn: http.client.HTTPConnection) -> tuple[list[list], float]:
    """Send N queries in one POST, return (list_of_id_lists, total_ms)."""
    body = json.dumps({"vecs": vecs, "limit": k, "mode": mode}).encode()
    t0 = time.perf_counter()
    try:
        conn.request("POST", "/vec_raw_batch", body, {"Content-Type": "application/json"})
        resp = conn.getresponse()
        results = json.loads(resp.read())
        ms = (time.perf_counter() - t0) * 1000
        id_lists = [[h.get("id") or h.get("doc_id") for h in hits] for hits in results]
        return id_lists, ms
    except Exception as e:
        return [[] for _ in vecs], (time.perf_counter() - t0) * 1000


# ── Phase A: ultra /vec_raw modes (single-query, keepalive) ───────────────────
def bench_ultra_raw(q_vecs_n: np.ndarray, gt_ids: list, mode: str) -> dict:
    label = f"ultra_raw ({mode})"
    if not ultra_ping():
        return {"engine": label, "available": False, "reason": f"ultra not at {ULTRA_URL}"}

    n = len(q_vecs_n)
    conn = _make_conn()
    # warmup
    for i in range(min(WARMUP, n)):
        ultra_raw_search(q_vecs_n[i].tolist(), K10, mode, conn)

    latencies, res10, res100 = [], [], []
    for i in range(n):
        ids100, ms = ultra_raw_search(q_vecs_n[i].tolist(), K100, mode, conn)
        latencies.append(ms)
        res10.append(ids100[:K10])
        res100.append(ids100)
    conn.close()

    st = pstats(latencies)
    return {
        "engine": label,
        "available": True,
        "mode": mode,
        "n_queries": n,
        **st,
        "recall_at_10": compute_recall(res10, gt_ids, K10),
        "recall_at_100": compute_recall(res100, gt_ids, K100),
        "note": "stored query vecs → /vec_raw keepalive (no re-embed).",
    }


# ── Phase D: ultra /vec_raw_batch (batch=32, keepalive) ───────────────────────
def bench_ultra_raw_batch(q_vecs_n: np.ndarray, gt_ids: list, mode: str, batch_size: int = BATCH_SIZE) -> dict:
    label = f"ultra_raw_batch_b{batch_size} ({mode})"
    if not ultra_ping():
        return {"engine": label, "available": False, "reason": f"ultra not at {ULTRA_URL}"}

    n = len(q_vecs_n)
    conn = _make_conn()
    # warmup (one batch)
    warmup_vecs = [q_vecs_n[i].tolist() for i in range(min(batch_size, n))]
    ultra_raw_batch(warmup_vecs, K10, mode, conn)

    latencies_per_q, res10, res100 = [], [], []
    for start in range(0, n, batch_size):
        chunk = q_vecs_n[start:start + batch_size]
        vecs_list = [v.tolist() for v in chunk]
        id_lists, batch_ms = ultra_raw_batch(vecs_list, K100, mode, conn)
        per_q_ms = batch_ms / len(chunk)
        for ids100 in id_lists:
            latencies_per_q.append(per_q_ms)
            res10.append(ids100[:K10])
            res100.append(ids100)
    conn.close()

    st = pstats(latencies_per_q)
    return {
        "engine": label,
        "available": True,
        "mode": mode,
        "batch_size": batch_size,
        "n_queries": n,
        **st,
        "recall_at_10": compute_recall(res10, gt_ids, K10),
        "recall_at_100": compute_recall(res100, gt_ids, K100),
        "note": f"batch={batch_size} queries/POST, keepalive. Amortizes HTTP overhead.",
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

    # Phase D: ultra batch (new — amortizes HTTP overhead)
    print(f"\n=== Phase D: ultra /vec_raw_batch (batch={BATCH_SIZE}, keepalive) ===")
    if ultra_ping():
        for mode in ["strict", "binary_first"]:
            print(f"  mode={mode} batch={BATCH_SIZE}...", flush=True)
            r = bench_ultra_raw_batch(q_vecs_n, gt_ids, mode, BATCH_SIZE)
            all_results.append(r)
            if r["available"]:
                print(f"    p50={r['p50_ms']:.3f}ms QPS={r['qps_1c']:.0f} R@10={r['recall_at_10']:.4f} R@100={r['recall_at_100']:.4f}")
    else:
        print("  SKIP: synapse-ultra not running at", ULTRA_URL)

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
