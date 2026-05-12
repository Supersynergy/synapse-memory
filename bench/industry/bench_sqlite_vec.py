#!/usr/bin/env python3
"""
Engine E: sqlite-vec direct (brute-force KNN on brain.db)
Uses raw vector queries (no text embedding).
"""
import json, os, sqlite3, time, sys
from pathlib import Path

import sqlite_vec
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from lib_gt import load_gt, load_corpus, compute_recall, pstats

BRAIN_DB = Path(os.environ.get("BRAIN_DB", "~/.synapse/brain.db")).expanduser()
K10 = 10
K100 = 100
WARMUP = 3
N_LIMIT = 100  # brute-force is ~700ms/query on 168k — cap to keep runtime sane


def main():
    q_vecs, gt_ids, meta = load_gt()
    n = min(len(q_vecs), N_LIMIT)
    q_vecs = q_vecs[:n]
    gt_ids = gt_ids[:n]
    print(f"[sqlite-vec] {n} queries (capped), corpus={meta['n_corpus']}")

    try:
        db = sqlite3.connect(str(BRAIN_DB))
        db.enable_load_extension(True)
        sqlite_vec.load(db)
        db.enable_load_extension(False)
        cnt = db.execute("SELECT COUNT(*) FROM docs_vec_rowids").fetchone()[0]
        print(f"  corpus rows: {cnt}")
    except Exception as e:
        result = {"engine": "sqlite-vec (brute)", "available": False, "reason": str(e)}
        print(json.dumps(result, indent=2))
        return result

    # Normalize query vecs (sqlite-vec uses cosine if vecs are normalized)
    norms = np.linalg.norm(q_vecs, axis=1, keepdims=True)
    norms[norms == 0] = 1.0
    q_vecs_n = (q_vecs / norms).astype(np.float32)

    # Warmup
    for i in range(min(WARMUP, n)):
        qb = q_vecs_n[i].tobytes()
        db.execute(
            "SELECT id FROM docs_vec WHERE embedding MATCH ? AND k = ?",
            (qb, K10)
        ).fetchall()

    latencies, res10, res100 = [], [], []
    for i in range(n):
        qb = q_vecs_n[i].tobytes()
        t0 = time.perf_counter()
        rows = db.execute(
            "SELECT id FROM docs_vec WHERE embedding MATCH ? AND k = ?",
            (qb, K100)
        ).fetchall()
        ms = (time.perf_counter() - t0) * 1000
        ids = [r[0] for r in rows]
        latencies.append(ms)
        res10.append(ids[:K10])
        res100.append(ids)

    st = pstats(latencies)
    result = {
        "engine": "sqlite-vec (brute)",
        "available": True,
        "n_queries": n,
        "note": f"capped at {N_LIMIT} queries (700ms/q on 168k corpus)",
        **st,
        "recall_at_10": compute_recall(res10, gt_ids, K10),
        "recall_at_100": compute_recall(res100, gt_ids, K100),
    }
    print(f"  p50={st['p50_ms']:.1f}ms p95={st['p95_ms']:.1f}ms QPS={st['qps_1c']:.0f} R@10={result['recall_at_10']:.4f}")
    print(json.dumps(result, indent=2))
    return result


if __name__ == "__main__":
    main()
