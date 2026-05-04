#!/usr/bin/env python3
"""Stack D: synapse-ultra (Team B crate). Skips gracefully if absent.

Detection order:
  1. HTTP endpoint at SYNAPSE_ULTRA_URL (default http://localhost:9478)
  2. Unix socket at SYNAPSE_ULTRA_SOCK
"""
import json, os, time, statistics
from pathlib import Path
from urllib.request import urlopen
from urllib.parse import urlencode
from urllib.error import URLError

ULTRA_URL = os.environ.get("SYNAPSE_ULTRA_URL", "http://localhost:9478")
ULTRA_SOCK = os.environ.get("SYNAPSE_ULTRA_SOCK", "/tmp/synapse-ultra.sock")
LIMIT = 10
WARM_ITERS = 100


def _http_search(q, limit=LIMIT):
    params = urlencode({"q": q, "k": limit})
    t0 = time.perf_counter()
    with urlopen(f"{ULTRA_URL}/vec?{params}", timeout=5) as r:
        data = json.loads(r.read())
    elapsed = (time.perf_counter() - t0) * 1000
    if isinstance(data, list):
        ids = [hit["id"] for hit in data]
    else:
        ids = [hit["id"] for hit in data.get("results", [])]
    return ids, elapsed


def _detect():
    try:
        with urlopen(f"{ULTRA_URL}/ping", timeout=2):
            return "http"
    except (URLError, OSError):
        pass
    if Path(ULTRA_SOCK).exists():
        return "sock"
    return None


def run(queries):
    mode = _detect()
    if mode is None:
        crate_path = Path(__file__).parent.parent.parent / "crates" / "synapse-ultra"
        reason = "synapse-ultra crate absent" if not crate_path.exists() else (
            f"crate exists at {crate_path} but daemon not running "
            f"(tried {ULTRA_URL} and {ULTRA_SOCK})"
        )
        return {"stack": "D", "available": False, "reason": reason}

    search = _http_search  # only HTTP mode implemented for now

    cold_times = []
    for q in queries:
        try:
            _, ms = search(q)
            cold_times.append(ms)
        except Exception as e:
            return {"stack": "D", "available": False, "reason": str(e)}

    warm_times = []
    for _ in range(WARM_ITERS):
        _, ms = search(queries[0])
        warm_times.append(ms)

    baseline_ids = {q: search(q)[0] for q in queries}

    def stats(ts):
        ts_s = sorted(ts)
        n = len(ts_s)
        return {
            "p50": ts_s[int(n * 0.50)],
            "p95": ts_s[int(n * 0.95)],
            "p99": ts_s[int(n * 0.99)],
            "mean": statistics.mean(ts_s),
            "qps": 1000.0 / statistics.mean(ts_s),
        }

    return {
        "stack": "D",
        "label": "synapse-ultra (Team B crate)",
        "available": True,
        "cold": stats(cold_times),
        "warm": stats(warm_times),
        "baseline_ids": baseline_ids,
    }


if __name__ == "__main__":
    queries_file = Path(__file__).parent / "queries.txt"
    queries = [l.strip() for l in queries_file.read_text().splitlines() if l.strip()]
    result = run(queries)
    print(json.dumps(result, indent=2))
