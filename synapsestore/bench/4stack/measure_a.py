#!/usr/bin/env python3
"""Stack A: sqlite-vec brute-force via synapsed Unix-socket msgpack daemon."""
import socket, struct, json, time, sys, os
from pathlib import Path

try:
    import msgpack
except ImportError:
    sys.exit("ERROR: msgpack not installed — pip install msgpack")

SOCK_PATH = os.environ.get("SYNAPSED_SOCK", "/tmp/synapse.sock")
LIMIT = int(os.environ.get("BENCH_LIMIT", "10"))
WARM_ITERS = int(os.environ.get("WARM_ITERS", "100"))
COLD_ITERS = int(os.environ.get("COLD_ITERS", "1"))


def _frame(req: dict) -> bytes:
    body = msgpack.packb(req, use_bin_type=True)
    return struct.pack("<I", len(body)) + body


def _recv(sock) -> dict:
    raw_len = b""
    while len(raw_len) < 4:
        chunk = sock.recv(4 - len(raw_len))
        if not chunk:
            raise ConnectionError("socket closed")
        raw_len += chunk
    (n,) = struct.unpack("<I", raw_len)
    body = b""
    while len(body) < n:
        chunk = sock.recv(n - len(body))
        if not chunk:
            raise ConnectionError("socket closed mid-body")
        body += chunk
    return msgpack.unpackb(body, raw=False)


def search(q: str, limit: int = LIMIT) -> tuple[list[int], float]:
    req = {
        "op": "Search",
        "args": {"mode": "Hybrid", "q": q, "limit": limit, "embed_query": True},
    }
    t0 = time.perf_counter()
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.connect(SOCK_PATH)
            s.sendall(_frame(req))
            resp = _recv(s)
    except (ConnectionRefusedError, FileNotFoundError) as e:
        raise RuntimeError(f"synapsed not reachable at {SOCK_PATH}: {e}")
    elapsed = (time.perf_counter() - t0) * 1000
    hits = resp.get("Hits", []) if isinstance(resp, dict) else []
    ids = [h["id"] if isinstance(h, dict) else h[0] for h in hits]
    return ids, elapsed


def run(queries: list[str]) -> dict:
    if not Path(SOCK_PATH).exists():
        return {"stack": "A", "available": False, "reason": f"socket missing: {SOCK_PATH}"}

    # cold run — 1 pass
    cold_times = []
    for q in queries:
        try:
            _, ms = search(q)
            cold_times.append(ms)
        except RuntimeError as e:
            return {"stack": "A", "available": False, "reason": str(e)}

    # warm run — WARM_ITERS passes, first query only (embed cache should be hot)
    warm_times = []
    for _ in range(WARM_ITERS):
        _, ms = search(queries[0])
        warm_times.append(ms)

    # baseline IDs for recall (cold, all queries)
    baseline = {}
    for q in queries:
        ids, _ = search(q)
        baseline[q] = ids

    import statistics
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
        "stack": "A",
        "label": "sqlite-vec brute-force (synapsed)",
        "available": True,
        "cold": stats(cold_times),
        "warm": stats(warm_times),
        "baseline_ids": baseline,
        "n_queries_cold": len(cold_times),
        "n_iters_warm": WARM_ITERS,
    }


if __name__ == "__main__":
    queries_file = Path(__file__).parent / "queries.txt"
    queries = [l.strip() for l in queries_file.read_text().splitlines() if l.strip()]
    result = run(queries)
    print(json.dumps(result, indent=2))
