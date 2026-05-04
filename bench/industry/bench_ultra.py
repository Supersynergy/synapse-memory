#!/usr/bin/env python3
"""
Engine A: synapse-ultra (HTTP, modes: binary_first + strict)

- Uses text queries (ultra embeds internally)
- Recall computed: strict mode = oracle, binary_first vs oracle
- Also reports ultra-strict vs brute-force GT recall
"""
import json, os, time, sys
from pathlib import Path
from urllib.request import urlopen
from urllib.parse import urlencode
from urllib.error import URLError

sys.path.insert(0, str(Path(__file__).parent))
from lib_gt import load_gt, load_texts, compute_recall, pstats

ULTRA_URL = os.environ.get("ULTRA_URL", "http://127.0.0.1:9478")
K10 = 10
K100 = 100
WARMUP = 10


def _ping():
    try:
        with urlopen(f"{ULTRA_URL}/ping", timeout=2):
            return True
    except Exception:
        return False


def _search(text: str, k: int, mode: str) -> tuple[list, float]:
    params = urlencode({"q": text, "limit": k, "mode": mode})
    t0 = time.perf_counter()
    try:
        with urlopen(f"{ULTRA_URL}/vec?{params}", timeout=10) as r:
            data = json.loads(r.read())
        ms = (time.perf_counter() - t0) * 1000
        # Handle both formats: list of {id,score} or {results:[...]}
        hits = data if isinstance(data, list) else data.get("results", [])
        ids = [h.get("id") or h.get("doc_id") for h in hits if h.get("id") or h.get("doc_id")]
        return ids, ms
    except Exception as e:
        return [], (time.perf_counter() - t0) * 1000


def bench_mode(texts, gt_ids, mode: str) -> tuple[dict, list, list]:
    label = f"ultra ({mode})"
    if not _ping():
        return {"engine": label, "available": False, "reason": f"ultra not reachable at {ULTRA_URL}"}, [], []

    n = len(texts)
    for i in range(min(WARMUP, n)):
        _search(texts[i], K10, mode)

    latencies, res10, res100 = [], [], []
    for text in texts:
        ids100, ms = _search(text, K100, mode)
        latencies.append(ms)
        res10.append(ids100[:K10])
        res100.append(ids100)

    st = pstats(latencies)
    note = ("Recall vs brute-force GT may be low: ultra re-embeds text queries; "
            "stored embeddings may differ from re-embedded. "
            "self_recall (binary_first vs strict) is the meaningful ANN metric.")
    return {
        "engine": label,
        "available": True,
        "mode": mode,
        "n_queries": n,
        **st,
        "recall_at_10_vs_gt": compute_recall(res10, gt_ids, K10),
        "recall_at_100_vs_gt": compute_recall(res100, gt_ids, K100),
        "note": note,
    }, res10, res100


def main():
    q_vecs, gt_ids, meta = load_gt()
    texts, _ = load_texts()
    print(f"[ultra] {len(texts)} queries, corpus={meta['n_corpus']}")

    results = []
    strict_res10, strict_res100 = [], []
    for mode in ["strict", "binary_first"]:
        print(f"  mode={mode}...", flush=True)
        r, res10, res100 = bench_mode(texts, gt_ids, mode)
        if mode == "strict" and r.get("available"):
            strict_res10, strict_res100 = res10, res100
        if mode == "binary_first" and r.get("available") and strict_res10:
            r["self_recall_at_10"] = compute_recall(res10, strict_res10, K10)
            r["self_recall_at_100"] = compute_recall(res100, strict_res100, K100)
        results.append(r)
        if r["available"]:
            sr = r.get("self_recall_at_10", r.get("recall_at_10_vs_gt", 0))
            print(f"    p50={r['p50_ms']:.1f}ms p95={r['p95_ms']:.1f}ms QPS={r['qps_1c']:.0f} recall@10(self)={sr:.4f}")
        else:
            print(f"    SKIP: {r.get('reason')}")

    print(json.dumps(results, indent=2))
    return results


if __name__ == "__main__":
    main()
