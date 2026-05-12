#!/usr/bin/env python3.13
"""Top5 SQLite-family deep bench (8 dimensions). Output line-buffered."""
import time, sqlite3, random, socket, struct, msgpack, threading
import apsw

BRAIN = "/tmp/brain_bench.db"
N_LOOK = 1000
N_FTS = 50

OUT = open("/tmp/bench_top5.txt", "w", buffering=1)
def log(s): OUT.write(s + "\n"); OUT.flush(); print(s, flush=True)

random.seed(42)
con0 = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
all_ids = [r[0] for r in con0.execute("SELECT rowid FROM docs LIMIT 100000").fetchall()]
ids = random.sample(all_ids, N_LOOK)
con0.close()

GBSQL = "SELECT substr(uri,1,30) p, COUNT(*) c FROM docs GROUP BY p ORDER BY c DESC LIMIT 5"
fts_q = ["synapse", "rabitq", "benchmark", "python", "vector", "milvus",
         "lancedb", "qdrant", "embedding", "claude"] * 5

results = {}  # tool -> {test: ms}
def add(tool, test, ms):
    results.setdefault(tool, {})[test] = ms

def run(tool, test, fn, runs=3):
    times = []
    for _ in range(runs):
        try:
            t0 = time.time(); fn(); times.append((time.time() - t0) * 1000)
        except Exception as e:
            log(f"  {tool} {test}: ERROR {str(e)[:60]}")
            return
    ms = min(times)
    add(tool, test, ms)
    log(f"  {tool:20s} {test:25s} {ms:8.2f} ms (best of {runs})")

log(f"=== Top5 SQLite Deep Bench (brain.db 177k docs) ===\n")

# === T1: Connect+close cycles ===
log("T1: 100 connect+close cycles")
def t1_stdlib():
    for _ in range(100):
        c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True); c.close()
run("sqlite-stdlib", "T1 conn cycles", t1_stdlib)

def t1_apsw():
    for _ in range(100):
        c = apsw.Connection(BRAIN, flags=apsw.SQLITE_OPEN_READONLY); c.close()
run("apsw", "T1 conn cycles", t1_apsw)

try:
    import libsql_experimental as libsql
    def t1_libsql():
        for _ in range(100):
            c = libsql.connect(BRAIN); del c
    run("libsql", "T1 conn cycles", t1_libsql)
except Exception as e:
    log(f"  libsql skip T1: {str(e)[:60]}")

import duckdb
def t1_duckdb():
    for _ in range(100):
        c = duckdb.connect(BRAIN, read_only=True); c.close()
run("duckdb", "T1 conn cycles", t1_duckdb)

def t1_synx():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(2)
    for _ in range(20):  # fewer iterations to avoid concurrent-storm with telepathy
        s.close()
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(2)
        s.connect('/tmp/synapse.sock')
        req = msgpack.packb({'op': 'ping'})
        s.sendall(struct.pack('>I', len(req)) + req)
        h = s.recv(4)
        if len(h) != 4: break
        n = struct.unpack('>I', h)[0]; d = b''
        while len(d) < n: d += s.recv(n - len(d))
    s.close()
run("synx-daemon", "T1 conn cycles (×20)", t1_synx, runs=1)

# === T2: Single rowid lookup ×1000 ===
log("\nT2: 1000 rowid lookups (per-row)")
def t2_stdlib():
    c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
    for i in ids: c.execute("SELECT uri FROM docs WHERE rowid=?", (i,)).fetchone()
    c.close()
run("sqlite-stdlib", "T2 1000 lookups", t2_stdlib)

def t2_apsw():
    c = apsw.Connection(BRAIN, flags=apsw.SQLITE_OPEN_READONLY)
    for i in ids: list(c.execute("SELECT uri FROM docs WHERE rowid=?", (i,)))
    c.close()
run("apsw", "T2 1000 lookups", t2_apsw)

# duckdb skipped (hung previously)
add("duckdb", "T2 1000 lookups", -1)

# === T3: IN-batch lookup ===
log("\nT3: 1000 ids batched in single IN clause")
def t3_stdlib():
    c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
    ph = ",".join("?" * len(ids))
    list(c.execute(f"SELECT uri FROM docs WHERE rowid IN ({ph})", ids))
    c.close()
run("sqlite-stdlib", "T3 IN batch", t3_stdlib)

def t3_apsw():
    c = apsw.Connection(BRAIN, flags=apsw.SQLITE_OPEN_READONLY)
    ph = ",".join("?" * len(ids))
    list(c.execute(f"SELECT uri FROM docs WHERE rowid IN ({ph})", ids))
    c.close()
run("apsw", "T3 IN batch", t3_apsw)

def t3_duckdb():
    c = duckdb.connect(BRAIN, read_only=True)
    ph = ",".join("?" * len(ids))
    list(c.execute(f"SELECT uri FROM docs WHERE rowid IN ({ph})", ids).fetchall())
    c.close()
run("duckdb", "T3 IN batch", t3_duckdb)

# === T4: GROUP BY 177k ===
log("\nT4: GROUP BY on 177k rows")
def t4_stdlib():
    c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True); list(c.execute(GBSQL)); c.close()
run("sqlite-stdlib", "T4 GROUP BY", t4_stdlib)

def t4_apsw():
    c = apsw.Connection(BRAIN, flags=apsw.SQLITE_OPEN_READONLY); list(c.execute(GBSQL)); c.close()
run("apsw", "T4 GROUP BY", t4_apsw)

def t4_duckdb():
    c = duckdb.connect(BRAIN, read_only=True); c.execute(GBSQL).fetchall(); c.close()
run("duckdb", "T4 GROUP BY", t4_duckdb)

# === T5: FTS5 50 queries ===
log("\nT5: FTS5 50 queries")
def t5_stdlib():
    c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
    for q in fts_q[:50]: list(c.execute("SELECT rowid FROM docs_fts WHERE docs_fts MATCH ? LIMIT 10", (q,)))
    c.close()
run("sqlite-stdlib", "T5 FTS5 50q", t5_stdlib)

def t5_apsw():
    c = apsw.Connection(BRAIN, flags=apsw.SQLITE_OPEN_READONLY)
    for q in fts_q[:50]: list(c.execute("SELECT rowid FROM docs_fts WHERE docs_fts MATCH ? LIMIT 10", (q,)))
    c.close()
run("apsw", "T5 FTS5 50q", t5_apsw)

def t5_synx():
    for q in fts_q[:50]:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect('/tmp/synapse.sock')
        req = msgpack.packb({'op': 'search', 'q': q, 'limit': 10})
        s.sendall(struct.pack('>I', len(req)) + req)
        h = s.recv(4); n = struct.unpack('>I', h)[0]; d = b''
        while len(d) < n: d += s.recv(n - len(d))
        s.close()
run("synx-daemon", "T5 FTS5 50q", t5_synx, runs=1)

# === T6: Vec hybrid via daemon ===
log("\nT6: Hybrid vec+FTS search 50 queries (synx daemon only)")
def t6_synx():
    for q in fts_q[:50]:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect('/tmp/synapse.sock')
        req = msgpack.packb({'op': 'hybrid', 'q': q, 'limit': 10})
        s.sendall(struct.pack('>I', len(req)) + req)
        h = s.recv(4); n = struct.unpack('>I', h)[0]; d = b''
        while len(d) < n: d += s.recv(n - len(d))
        s.close()
run("synx-daemon", "T6 hybrid vec+FTS 50q", t6_synx, runs=1)

# === T7: Tuned PRAGMA GROUP BY ===
log("\nT7: GROUP BY with mmap+cache PRAGMA tuning")
def t7_stdlib():
    c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
    c.execute("PRAGMA cache_size=-128000"); c.execute("PRAGMA mmap_size=536870912")
    list(c.execute(GBSQL)); c.close()
run("sqlite-stdlib", "T7 GROUP BY tuned", t7_stdlib)

def t7_apsw():
    c = apsw.Connection(BRAIN, flags=apsw.SQLITE_OPEN_READONLY)
    c.execute("PRAGMA cache_size=-128000"); c.execute("PRAGMA mmap_size=536870912")
    list(c.execute(GBSQL)); c.close()
run("apsw", "T7 GROUP BY tuned", t7_apsw)

# === T8: 8 concurrent threads ===
log("\nT8: 8 concurrent threads, each 100 lookups")
def t8_stdlib():
    def worker():
        c = sqlite3.connect(f"file:{BRAIN}?mode=ro", uri=True)
        for i in random.sample(ids, 100):
            c.execute("SELECT uri FROM docs WHERE rowid=?", (i,)).fetchone()
        c.close()
    ts = [threading.Thread(target=worker) for _ in range(8)]
    [t.start() for t in ts]; [t.join() for t in ts]
run("sqlite-stdlib", "T8 8 concurrent", t8_stdlib)

def t8_apsw():
    def worker():
        c = apsw.Connection(BRAIN, flags=apsw.SQLITE_OPEN_READONLY)
        for i in random.sample(ids, 100):
            list(c.execute("SELECT uri FROM docs WHERE rowid=?", (i,)))
        c.close()
    ts = [threading.Thread(target=worker) for _ in range(8)]
    [t.start() for t in ts]; [t.join() for t in ts]
run("apsw", "T8 8 concurrent", t8_apsw)

# Daemon T8 skipped — concurrent socket storm hangs (single-thread serialization)
add("synx-daemon", "T8 8 concurrent", -1)

# === Summary ===
log("\n=== SUMMARY ===")
all_tools = list(results.keys())
all_tests = sorted({t for r in results.values() for t in r})
log(f"\n| Test | " + " | ".join(all_tools) + " |")
log("|" + "---|" * (len(all_tools)+1))
for t in all_tests:
    row = f"| {t} | "
    best = min((results[tl].get(t, 99999), tl) for tl in all_tools if results[tl].get(t, -1) > 0)[1]
    for tl in all_tools:
        v = results[tl].get(t, -1)
        cell = f"{v:.2f}ms" if v > 0 else "—"
        if tl == best and v > 0: cell = f"**{cell}**"
        row += cell + " | "
    log(row)

# Avg rank
log("\n=== Avg rank across tests (lower=better) ===")
ranks = {tl: [] for tl in all_tools}
for t in all_tests:
    valid = [(tl, results[tl].get(t, -1)) for tl in all_tools if results[tl].get(t, -1) > 0]
    valid.sort(key=lambda x: x[1])
    for i, (tl, _) in enumerate(valid):
        ranks[tl].append(i + 1)
for tl in sorted(all_tools, key=lambda x: sum(ranks[x])/max(len(ranks[x]),1) if ranks[x] else 99):
    if ranks[tl]:
        log(f"  {tl:20s} avg-rank={sum(ranks[tl])/len(ranks[tl]):.2f}  (n={len(ranks[tl])})")

OUT.close()
print("BENCH DONE")
