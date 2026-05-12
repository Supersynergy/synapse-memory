#!/usr/bin/env python3.13
"""Minimal bench, write each result immediately."""
import time, sqlite3, sys

OUT = open("/tmp/bench_out.txt", "w", buffering=1)  # line-buffered

def log(msg):
    OUT.write(msg + "\n")
    OUT.flush()
    print(msg)

BRAIN = "/tmp/brain_bench.db"
log(f"=== bench start {time.strftime('%H:%M:%S')} ===")

# Test 1: sqlite3 connect+count
t0 = time.time()
c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
n = c.execute("SELECT COUNT(*) FROM docs").fetchone()[0]
c.close()
log(f"sqlite3 COUNT(*) on {n} docs: {(time.time()-t0)*1000:.1f}ms")

# Test 2: GROUP BY
t0 = time.time()
c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
list(c.execute("SELECT substr(uri,1,30) p, COUNT(*) c FROM docs GROUP BY p ORDER BY c DESC LIMIT 5"))
c.close()
log(f"sqlite3 GROUP BY: {(time.time()-t0)*1000:.1f}ms")

# Test 3: apsw GROUP BY
import apsw
t0 = time.time()
c = apsw.Connection(BRAIN, flags=apsw.SQLITE_OPEN_READONLY)
list(c.execute("SELECT substr(uri,1,30) p, COUNT(*) c FROM docs GROUP BY p ORDER BY c DESC LIMIT 5"))
c.close()
log(f"apsw GROUP BY: {(time.time()-t0)*1000:.1f}ms")

# Test 4: 100 lookups stdlib
import random
random.seed(42)
c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
ids = random.sample([r[0] for r in c.execute("SELECT rowid FROM docs LIMIT 50000").fetchall()], 100)
c.close()
log(f"sample {len(ids)} ids ready")

t0 = time.time()
c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
for i in ids: c.execute("SELECT uri FROM docs WHERE rowid=?", (i,)).fetchone()
c.close()
log(f"sqlite3 100 lookups: {(time.time()-t0)*1000:.1f}ms")

t0 = time.time()
c = apsw.Connection(BRAIN, flags=apsw.SQLITE_OPEN_READONLY)
for i in ids: list(c.execute("SELECT uri FROM docs WHERE rowid=?", (i,)))
c.close()
log(f"apsw 100 lookups: {(time.time()-t0)*1000:.1f}ms")

# Test 5: synx daemon ping (live)
import socket, struct, msgpack
t0 = time.time()
for _ in range(100):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect('/tmp/synapse.sock')
    req = msgpack.packb({'op': 'ping'})
    s.sendall(struct.pack('>I', len(req)) + req)
    h = s.recv(4); n = struct.unpack('>I', h)[0]; d = b''
    while len(d) < n: d += s.recv(n - len(d))
    s.close()
log(f"synx daemon 100 pings: {(time.time()-t0)*1000:.1f}ms")

# Test 6: synx daemon hybrid search
t0 = time.time()
for _ in range(10):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect('/tmp/synapse.sock')
    req = msgpack.packb({'op': 'hybrid', 'q': 'test', 'limit': 10})
    s.sendall(struct.pack('>I', len(req)) + req)
    h = s.recv(4); n = struct.unpack('>I', h)[0]; d = b''
    while len(d) < n: d += s.recv(n - len(d))
    s.close()
log(f"synx daemon 10 hybrid searches: {(time.time()-t0)*1000:.1f}ms")

OUT.close()
print("done")
