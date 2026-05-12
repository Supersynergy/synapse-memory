#!/usr/bin/env python3
"""
zero_overhead_bench.py — benchmark HTTP vs UDS+msgpack vs UDS+bincode vs FFI
Outputs results/2026-05-05/zero_overhead_paths.md
"""
import struct
import socket
import time
import statistics
import random
import threading
import os
import sys
import ctypes
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

try:
    import requests
except ImportError:
    print("pip install requests"); sys.exit(1)
try:
    import msgpack
except ImportError:
    print("pip install msgpack"); sys.exit(1)
try:
    import numpy as np
except ImportError:
    print("pip install numpy"); sys.exit(1)

# ── Config ─────────────────────────────────────────────────────────────────────
DIM = 384
K = 10
N_QUERIES = 1000
CONCURRENCY = 12
HTTP_URL = "http://127.0.0.1:9477/search"
UDS_MSGPACK = "/tmp/synapse-ultra.sock"
UDS_BINCODE = "/tmp/synapse-ultra-vec.sock"
SNAP_PATH = os.path.expanduser("~/.synapse/ultra_matrix.bin")
DYLIB_PATH = Path(__file__).parent.parent / "synapsestore/crates/synapse-ultra/../../target/release/libsynapse_ultra.dylib"
if not DYLIB_PATH.exists():
    DYLIB_PATH = Path(__file__).parent.parent / "target/release/libsynapse_ultra.dylib"

# Pre-generate random normalized queries
rng = random.Random(42)
def rand_vec():
    v = [rng.gauss(0, 1) for _ in range(DIM)]
    n = sum(x*x for x in v)**0.5
    return [x/n for x in v]

QUERIES = [rand_vec() for _ in range(N_QUERIES)]

# ── Helpers ────────────────────────────────────────────────────────────────────
def percentiles(times_us):
    s = sorted(times_us)
    n = len(s)
    p50 = s[n//2]
    p95 = s[int(n*0.95)]
    p99 = s[int(n*0.99)]
    return p50, p95, p99

def qps(times_us, total_wall_s):
    return len(times_us) / total_wall_s

# ── Path 1: HTTP (no keepalive — fresh connection each) ──────────────────────
def bench_http_nokeepalive():
    times = []
    t0 = time.perf_counter()
    for q in QUERIES:
        t = time.perf_counter()
        r = requests.post(HTTP_URL, json={"q": "bench", "limit": K}, timeout=5)
        times.append((time.perf_counter() - t) * 1e6)
    wall = time.perf_counter() - t0
    return times, wall

# ── Path 2: HTTP keepalive ─────────────────────────────────────────────────────
def bench_http_keepalive():
    session = requests.Session()
    times = []
    t0 = time.perf_counter()
    for q in QUERIES:
        t = time.perf_counter()
        r = session.post(HTTP_URL, json={"q": "bench", "limit": K}, timeout=5)
        times.append((time.perf_counter() - t) * 1e6)
    wall = time.perf_counter() - t0
    return times, wall

# ── Path 3: UDS + msgpack (existing, text query) ─────────────────────────────
def _msgpack_query(q_str, limit, mode="binary_first"):
    payload = msgpack.packb({"op": "Search", "args": {"q": q_str, "limit": limit, "mode": mode}}, use_bin_type=True)
    lp = struct.pack("<I", len(payload))
    return lp + payload

def bench_uds_msgpack():
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(UDS_MSGPACK)
    times = []
    t0 = time.perf_counter()
    for i, q in enumerate(QUERIES):
        frame = _msgpack_query("bench query", K)
        t = time.perf_counter()
        sock.sendall(frame)
        hdr = sock.recv(4)
        rlen = struct.unpack("<I", hdr)[0]
        resp = b""
        while len(resp) < rlen:
            chunk = sock.recv(rlen - len(resp))
            if not chunk:
                break
            resp += chunk
        times.append((time.perf_counter() - t) * 1e6)
    wall = time.perf_counter() - t0
    sock.close()
    return times, wall

# ── Path 4: UDS + bincode (new vec path) ─────────────────────────────────────
def _encode_vec_query(vec, limit, mode=1):
    # bincode encoding of VecRawQuery { vec: Vec<f32>, limit: u32, mode: u8 }
    # bincode default: little-endian, u64 length prefix for Vec
    vec_bytes = struct.pack(f"<Q{len(vec)}f", len(vec), *vec)
    rest = struct.pack("<IB", limit, mode)
    return vec_bytes + rest

def bench_uds_bincode_1c():
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(UDS_BINCODE)
    times = []
    t0 = time.perf_counter()
    for q in QUERIES:
        payload = _encode_vec_query(q, K)
        lp = struct.pack("<I", len(payload))
        t = time.perf_counter()
        sock.sendall(lp + payload)
        hdr = sock.recv(4)
        rlen = struct.unpack("<I", hdr)[0]
        resp = b""
        while len(resp) < rlen:
            chunk = sock.recv(rlen - len(resp))
            if not chunk:
                break
            resp += chunk
        times.append((time.perf_counter() - t) * 1e6)
    wall = time.perf_counter() - t0
    sock.close()
    return times, wall

def _bincode_worker(queries):
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(UDS_BINCODE)
    times = []
    for q in queries:
        payload = _encode_vec_query(q, K)
        lp = struct.pack("<I", len(payload))
        t = time.perf_counter()
        sock.sendall(lp + payload)
        hdr = sock.recv(4)
        rlen = struct.unpack("<I", hdr)[0]
        resp = b""
        while len(resp) < rlen:
            chunk = sock.recv(rlen - len(resp))
            if not chunk:
                break
            resp += chunk
        times.append((time.perf_counter() - t) * 1e6)
    sock.close()
    return times

def bench_uds_bincode_12c():
    chunk_size = N_QUERIES // CONCURRENCY
    chunks = [QUERIES[i*chunk_size:(i+1)*chunk_size] for i in range(CONCURRENCY)]
    all_times = []
    t0 = time.perf_counter()
    with ThreadPoolExecutor(max_workers=CONCURRENCY) as ex:
        futures = [ex.submit(_bincode_worker, chunk) for chunk in chunks]
        for f in futures:
            all_times.extend(f.result())
    wall = time.perf_counter() - t0
    return all_times, wall

# ── Path 5: FFI direct ────────────────────────────────────────────────────────
def bench_ffi():
    dylib = str(DYLIB_PATH)
    if not Path(dylib).exists():
        print(f"WARN: dylib not found at {dylib}, skipping FFI bench")
        return None, None

    lib = ctypes.CDLL(dylib)
    lib.synapse_open.restype = ctypes.c_void_p
    lib.synapse_open.argtypes = [ctypes.c_char_p]
    lib.synapse_close.argtypes = [ctypes.c_void_p]
    lib.synapse_search_raw.restype = ctypes.c_int32
    lib.synapse_search_raw.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_float), ctypes.c_size_t,
        ctypes.c_size_t, ctypes.c_uint8,
        ctypes.POINTER(ctypes.c_int64), ctypes.POINTER(ctypes.c_float)
    ]

    handle = lib.synapse_open(SNAP_PATH.encode())
    if not handle:
        print("WARN: synapse_open returned null — snapshot not found?")
        return None, None

    out_ids = (ctypes.c_int64 * K)()
    out_scores = (ctypes.c_float * K)()

    times = []
    t0 = time.perf_counter()
    for q in QUERIES:
        arr = (ctypes.c_float * DIM)(*q)
        t = time.perf_counter()
        n = lib.synapse_search_raw(handle, arr, DIM, K, 1, out_ids, out_scores)
        times.append((time.perf_counter() - t) * 1e6)
    wall = time.perf_counter() - t0
    lib.synapse_close(handle)
    return times, wall

# ── Main ───────────────────────────────────────────────────────────────────────
def run_bench(name, fn):
    print(f"\n{'='*50}\n{name}", flush=True)
    try:
        times, wall = fn()
        if times is None:
            return None
        p50, p95, p99 = percentiles(times)
        q = qps(times, wall)
        print(f"  p50={p50:.1f}µs  p95={p95:.1f}µs  p99={p99:.1f}µs  QPS={q:.0f}")
        return {"name": name, "p50": p50, "p95": p95, "p99": p99, "qps": q, "overhead": p50}
    except Exception as e:
        print(f"  ERROR: {e}")
        return None

def main():
    results = []

    # Skip HTTP if not reachable
    try:
        r = requests.get("http://127.0.0.1:9477/stats", timeout=1)
        has_http = True
    except Exception:
        has_http = False
        print("WARN: HTTP server not reachable at :9477 — skipping HTTP benchmarks")

    if has_http:
        results.append(run_bench("HTTP no-keepalive", bench_http_nokeepalive))
        results.append(run_bench("HTTP keepalive", bench_http_keepalive))

    # UDS msgpack (needs running daemon)
    try:
        r = run_bench("UDS+msgpack 1c (text query, existing)", bench_uds_msgpack)
        results.append(r)
    except Exception as e:
        print(f"UDS msgpack error: {e}")

    # UDS bincode (new vec path)
    try:
        r = run_bench("UDS+bincode 1c (vec, new)", bench_uds_bincode_1c)
        results.append(r)
        r12 = run_bench(f"UDS+bincode {CONCURRENCY}c (vec, new)", bench_uds_bincode_12c)
        results.append(r12)
    except Exception as e:
        print(f"UDS bincode error: {e}")

    # FFI (in-proc, no daemon needed)
    r = run_bench("FFI direct (in-proc, cdylib)", bench_ffi)
    results.append(r)

    # Write markdown table
    out_dir = Path(__file__).parent / "results/2026-05-05"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "zero_overhead_paths.md"

    rows = [r for r in results if r is not None]

    with open(out_path, "w") as f:
        f.write("# Zero-Overhead Paths — Benchmark Results\n\n")
        f.write(f"**Date**: 2026-05-05  **N**: {N_QUERIES} queries  **k**: {K}  **dim**: {DIM}\n\n")
        f.write("## Latency Table\n\n")
        f.write("| Path | p50 µs | p95 µs | p99 µs | QPS | Overhead vs FFI |\n")
        f.write("|------|--------|--------|--------|-----|----------------|\n")

        # FFI is the baseline
        ffi_p50 = next((r["p50"] for r in rows if "FFI" in r["name"]), None)

        for r in rows:
            overhead = f"{r['p50']/ffi_p50:.1f}×" if ffi_p50 else "—"
            f.write(f"| {r['name']} | {r['p50']:.1f} | {r['p95']:.1f} | {r['p99']:.1f} | {r['qps']:.0f} | {overhead} |\n")

        f.write("\n## Notes\n\n")
        f.write("- UDS+bincode path accepts pre-computed f32 vectors — no embedding overhead in measurement\n")
        f.write("- UDS+msgpack path sends text and embeds server-side — includes embedding time\n")
        f.write("- FFI is in-process: zero IPC, zero serialization, pure search kernel\n")
        f.write("- Darwin UDS: 2 kernel copies (send→kernel buf, kernel buf→recv). TCP localhost: same on loopback.\n")
        f.write("- If UDS doesn't beat HTTP+keepalive meaningfully: kernel copy cost dominates at this payload size\n")

    print(f"\nResults written to {out_path}")
    for r in rows:
        print(f"  {r['name']:45s} p50={r['p50']:7.1f}µs  QPS={r['qps']:.0f}")

if __name__ == "__main__":
    main()
