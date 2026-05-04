#!/usr/bin/env python3
"""Stack B: Python turbo daemon at :9477 (NumPy in-memory, warm ~0.6ms)."""
import json, time, sys, statistics
from pathlib import Path
from urllib.request import urlopen
from urllib.parse import urlencode
from urllib.error import URLError

BASE_URL = "http://localhost:9477"
LIMIT = 10
WARM_ITERS = 100
COLD_ITERS = 1


def search(q: str, mode: str = "hybrid", limit: int = LIMIT) -> tuple[list[int], float]:
    params = urlencode({"q": q, "limit": limit})
    url = f"{BASE_URL}/{mode}?{params}"
    t0 = time.perf_counter()
    with urlopen(url, timeout=10) as r:
        data = json.loads(r.read())
    elapsed = (time.perf_counter() - t0) * 1000
    ids = [hit["id"] for hit in data.get("results", [])]
    return ids, elapsed


def run(queries: list[str]) -> dict:
    # availability check
    try:
        with urlopen(f"{BASE_URL}/health", timeout=3) as r:
            health = json.loads(r.read())
    except (URLError, OSError) as e:
        return {"stack": "B", "available": False, "reason": str(e)}

    # cold run
    cold_times = []
    for q in queries:
        try:
            _, ms = search(q)
            cold_times.append(ms)
        except Exception as e:
            return {"stack": "B", "available": False, "reason": str(e)}

    # warm run
    warm_times = []
    for _ in range(WARM_ITERS):
        _, ms = search(queries[0])
        warm_times.append(ms)

    # IDs for recall comparison
    baseline_ids = {}
    for q in queries:
        ids, _ = search(q)
        baseline_ids[q] = ids

    def stats(ts):
        ts_s = sorted(ts)
        n = len(ts_s)
        return {
            "p50": ts_s[int(n * 0.50)],
            "p95": ts_s[int(n * 0.95)],
            "p99": ts_s[int(n * 0.99)],
            "mean": statistics.mean(ts_s),
            "qps": 1000.0 / statistics.mean(ts_s) if statistics.mean(ts_s) > 0 else 0,
        }

    return {
        "stack": "B",
        "label": "Python turbo daemon :9477 (NumPy)",
        "available": True,
        "cold": stats(cold_times),
        "warm": stats(warm_times),
        "baseline_ids": baseline_ids,
        "n_queries_cold": len(cold_times),
        "n_iters_warm": WARM_ITERS,
        "daemon_info": health,
    }


if __name__ == "__main__":
    queries_file = Path(__file__).parent / "queries.txt"
    queries = [l.strip() for l in queries_file.read_text().splitlines() if l.strip()]
    result = run(queries)
    print(json.dumps(result, indent=2))
