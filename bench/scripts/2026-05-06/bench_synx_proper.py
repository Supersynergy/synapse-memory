#!/usr/bin/env python3.13
"""Proper synx-daemon bench — correct protocol (little-endian, capitalized op)."""
import time, socket, struct, msgpack
from concurrent.futures import ThreadPoolExecutor

SOCK = '/tmp/synapse.sock'
OUT = open("/tmp/bench_synx.txt", "w", buffering=1)
def log(s): OUT.write(s + "\n"); OUT.flush(); print(s, flush=True)

def call(req, timeout=10):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect(SOCK)
        body = msgpack.packb(req)
        s.sendall(struct.pack("<I", len(body)) + body)  # LITTLE-ENDIAN
        hdr = b""
        while len(hdr) < 4:
            chunk = s.recv(4 - len(hdr))
            if not chunk: return None
            hdr += chunk
        n = struct.unpack("<I", hdr)[0]
        buf = b""
        while len(buf) < n:
            chunk = s.recv(min(n - len(buf), 65536))
            if not chunk: break
            buf += chunk
        return msgpack.unpackb(buf, raw=False) if len(buf) == n else None
    finally:
        s.close()

# Warm up
r = call({"op": "Ping"})
log(f"warm-up Ping: {r}")

# T1: 100 sequential pings
log("\n=== T1: Sequential Pings ×100 ===")
for runs in range(3):
    t0 = time.time()
    for _ in range(100):
        call({"op": "Ping"})
    el = (time.time() - t0) * 1000
    log(f"  run{runs+1}: {el:.1f}ms total = {el/100:.3f}ms/ping")

# T2: Hybrid search ×50
log("\n=== T2: Hybrid Search ×50 ===")
queries = ['synapse', 'rabitq', 'python', 'vector', 'test'] * 10
for runs in range(2):
    t0 = time.time()
    for q in queries:
        call({"op": "Search", "args": {"mode": "Hybrid", "q": q, "limit": 10, "embed_query": True}})
    el = (time.time() - t0) * 1000
    log(f"  run{runs+1}: {el:.1f}ms total = {el/50:.2f}ms/q")

# T3: FTS5 only ×50
log("\n=== T3: Search Fts ×50 ===")
for runs in range(2):
    t0 = time.time()
    for q in queries:
        call({"op": "Search", "args": {"mode": "Fts", "q": q, "limit": 10, "embed_query": False}})
    el = (time.time() - t0) * 1000
    log(f"  run{runs+1}: {el:.1f}ms total = {el/50:.2f}ms/q")

# T4: 8 concurrent × 25 pings
log("\n=== T4: 8 concurrent × 25 pings (200 ops) ===")
def worker(n):
    for _ in range(n): call({"op": "Ping"})
for runs in range(2):
    t0 = time.time()
    with ThreadPoolExecutor(max_workers=8) as ex:
        list(ex.map(worker, [25]*8))
    el = (time.time() - t0) * 1000
    log(f"  run{runs+1}: {el:.1f}ms total = {el/200:.3f}ms/ping concurrent")

# T5: 8 concurrent × 5 hybrid (40 ops)
log("\n=== T5: 8 concurrent × 5 Hybrid (40 ops) ===")
def worker_h(n):
    for _ in range(n): call({"op": "Search", "args": {"mode": "Hybrid", "q": "vector", "limit": 10, "embed_query": True}})
for runs in range(2):
    t0 = time.time()
    with ThreadPoolExecutor(max_workers=8) as ex:
        list(ex.map(worker_h, [5]*8))
    el = (time.time() - t0) * 1000
    log(f"  run{runs+1}: {el:.1f}ms total = {el/40:.2f}ms/q concurrent")

# T6: Stats
log("\n=== T6: Stats ×10 ===")
t0 = time.time()
for _ in range(10):
    call({"op": "Stats"})
log(f"  {(time.time()-t0)*100:.2f}ms/stats")

# T7: Mixed batch — 100 mixed ops in 1 thread
log("\n=== T7: 100 mixed ops sequential ===")
mix_ops = [
    {"op": "Ping"},
    {"op": "Stats"},
    {"op": "Search", "args": {"mode": "Fts", "q": "test", "limit": 5, "embed_query": False}},
] * 34
t0 = time.time()
for op in mix_ops[:100]: call(op)
log(f"  {(time.time()-t0)*1000:.1f}ms = {(time.time()-t0)*10:.2f}ms/op")

OUT.close()
print("DONE")
