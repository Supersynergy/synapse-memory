#!/usr/bin/env python3
"""Concurrent 12-worker bench for synapse-ultra /vec_raw and /vec_raw_batch."""
import json, os, time, sys, threading
import http.client
from pathlib import Path
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from lib_gt import load_gt, load_corpus, compute_recall, pstats

ULTRA_URL = os.environ.get("ULTRA_URL", "http://127.0.0.1:9478")
WORKERS = int(os.environ.get("WORKERS", "12"))
BATCH_SIZE = int(os.environ.get("ULTRA_BATCH_SIZE", "32"))
K = 10

def parse_host_port():
    url = ULTRA_URL.removeprefix("http://")
    if ":" in url:
        h, p = url.rsplit(":", 1)
        return h, int(p)
    return url, 80

HOST, PORT = parse_host_port()


def worker_single(q_vecs, k, mode, results, idx):
    conn = http.client.HTTPConnection(HOST, PORT, timeout=10)
    ids_all, latencies = [], []
    for vec in q_vecs:
        body = json.dumps({"vec": vec.tolist(), "limit": k, "mode": mode}).encode()
        t0 = time.perf_counter()
        conn.request("POST", "/vec_raw", body, {"Content-Type": "application/json"})
        resp = conn.getresponse()
        hits = json.loads(resp.read())
        ms = (time.perf_counter() - t0) * 1000
        latencies.append(ms)
        ids_all.append([h.get("id") or h.get("doc_id") for h in hits])
    conn.close()
    results[idx] = (ids_all, latencies)


def worker_batch(q_vecs, k, mode, batch_size, results, idx):
    conn = http.client.HTTPConnection(HOST, PORT, timeout=10)
    ids_all, latencies = [], []
    for start in range(0, len(q_vecs), batch_size):
        chunk = q_vecs[start:start + batch_size]
        body = json.dumps({"vecs": [v.tolist() for v in chunk], "limit": k, "mode": mode}).encode()
        t0 = time.perf_counter()
        conn.request("POST", "/vec_raw_batch", body, {"Content-Type": "application/json"})
        resp = conn.getresponse()
        results_batch = json.loads(resp.read())
        ms = (time.perf_counter() - t0) * 1000
        per_q = ms / len(chunk)
        for hits in results_batch:
            ids_all.append([h.get("id") or h.get("doc_id") for h in hits])
            latencies.append(per_q)
    conn.close()
    results[idx] = (ids_all, latencies)


def bench_concurrent(q_vecs_n, gt_ids, mode, endpoint="single", batch_size=32, n_workers=WORKERS):
    n = len(q_vecs_n)
    chunk = n // n_workers
    chunks = [q_vecs_n[i*chunk:(i+1)*chunk if i < n_workers-1 else n] for i in range(n_workers)]

    results = [None] * n_workers
    fn = worker_batch if endpoint == "batch" else worker_single

    # warmup: 1 worker, 5 queries
    warmup_conn = http.client.HTTPConnection(HOST, PORT, timeout=10)
    for i in range(5):
        body = json.dumps({"vec": q_vecs_n[i].tolist(), "limit": K, "mode": mode}).encode()
        warmup_conn.request("POST", "/vec_raw", body, {"Content-Type": "application/json"})
        warmup_conn.getresponse().read()
    warmup_conn.close()

    t_wall0 = time.perf_counter()
    threads = []
    for i in range(n_workers):
        if endpoint == "batch":
            t = threading.Thread(target=fn, args=(chunks[i], K, mode, batch_size, results, i))
        else:
            t = threading.Thread(target=fn, args=(chunks[i], K, mode, results, i))
        threads.append(t)
    for t in threads: t.start()
    for t in threads: t.join()
    t_wall = time.perf_counter() - t_wall0

    all_ids = []
    all_lat = []
    for ids_all, latencies in results:
        all_ids.extend(ids_all)
        all_lat.extend(latencies)

    recall = compute_recall(all_ids, gt_ids, K)
    total_q = len(all_ids)
    agg_qps = total_q / t_wall
    st = pstats(all_lat)
    return {
        "engine": f"concurrent_{n_workers}w_{endpoint} ({mode})",
        "workers": n_workers,
        "n_queries": total_q,
        "wall_s": round(t_wall, 3),
        "agg_qps": round(agg_qps, 0),
        **st,
        "recall_at_10": recall,
    }


def main():
    q_vecs, gt_ids, meta = load_gt()
    print(f"[concurrent bench] {len(q_vecs)} queries, corpus={meta['n_corpus']}, workers={WORKERS}")

    norms = np.linalg.norm(q_vecs, axis=1, keepdims=True)
    norms[norms == 0] = 1.0
    q_vecs_n = (q_vecs / norms).astype(np.float32)

    all_results = []
    for mode in ["strict", "binary_first"]:
        print(f"\n--- /vec_raw concurrent {WORKERS} workers, mode={mode} ---")
        r = bench_concurrent(q_vecs_n, gt_ids, mode, endpoint="single")
        all_results.append(r)
        print(f"  agg_qps={r['agg_qps']} p50={r['p50_ms']:.3f}ms R@10={r['recall_at_10']:.4f}")

    for mode in ["strict", "binary_first"]:
        print(f"\n--- /vec_raw_batch (b={BATCH_SIZE}) concurrent {WORKERS} workers, mode={mode} ---")
        r = bench_concurrent(q_vecs_n, gt_ids, mode, endpoint="batch", batch_size=BATCH_SIZE)
        all_results.append(r)
        print(f"  agg_qps={r['agg_qps']} p50={r['p50_ms']:.3f}ms R@10={r['recall_at_10']:.4f}")

    print("\n=== RESULTS JSON ===")
    print(json.dumps(all_results, indent=2))
    return all_results


if __name__ == "__main__":
    main()
