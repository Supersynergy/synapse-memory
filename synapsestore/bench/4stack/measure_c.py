#!/usr/bin/env python3
"""Stack C: synapse-core Step-5 ndarray build (CORE_TURBO=1 env, separate socket).

Gracefully skips if:
  - env CORE_TURBO=1 is not set, OR
  - socket SYNAPSED_TURBO_SOCK (/tmp/synapse-turbobench.sock default) is absent.
"""
import json, os, time, statistics, sys, struct
from pathlib import Path

SOCK_PATH = os.environ.get("SYNAPSED_TURBO_SOCK", "/tmp/synapse-turbobench.sock")
CORE_TURBO = os.environ.get("CORE_TURBO", "0") == "1"
LIMIT = 10
WARM_ITERS = 100

try:
    import msgpack
    HAS_MSGPACK = True
except ImportError:
    HAS_MSGPACK = False


def _frame(req):
    body = msgpack.packb(req, use_bin_type=True)
    return struct.pack("<I", len(body)) + body


def _recv(sock):
    import socket as _socket
    raw_len = b""
    while len(raw_len) < 4:
        c = sock.recv(4 - len(raw_len))
        if not c:
            raise ConnectionError("closed")
        raw_len += c
    (n,) = struct.unpack("<I", raw_len)
    body = b""
    while len(body) < n:
        c = sock.recv(n - len(body))
        if not c:
            raise ConnectionError("mid-body")
        body += c
    return msgpack.unpackb(body, raw=False)


def search(q, limit=LIMIT):
    import socket as _socket
    req = {"op": "Search", "args": {"mode": "Hybrid", "q": q, "limit": limit, "embed_query": True}}
    t0 = time.perf_counter()
    with _socket.socket(_socket.AF_UNIX, _socket.SOCK_STREAM) as s:
        s.connect(SOCK_PATH)
        s.sendall(_frame(req))
        resp = _recv(s)
    elapsed = (time.perf_counter() - t0) * 1000
    hits = resp.get("Hits", []) if isinstance(resp, dict) else []
    ids = [h["id"] if isinstance(h, dict) else h[0] for h in hits]
    return ids, elapsed


def run(queries):
    if not CORE_TURBO:
        return {
            "stack": "C",
            "available": False,
            "reason": "CORE_TURBO=1 not set — Step-5 ndarray build not requested",
        }
    if not HAS_MSGPACK:
        return {"stack": "C", "available": False, "reason": "msgpack not installed"}
    if not Path(SOCK_PATH).exists():
        return {
            "stack": "C",
            "available": False,
            "reason": f"Step-5 turbo socket absent: {SOCK_PATH}",
        }

    cold_times = []
    for q in queries:
        try:
            _, ms = search(q)
            cold_times.append(ms)
        except Exception as e:
            return {"stack": "C", "available": False, "reason": str(e)}

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
        "stack": "C",
        "label": "synapse-core Step-5 ndarray (CORE_TURBO)",
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
