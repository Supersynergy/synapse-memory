#!/usr/bin/env python3
"""Mini OLTP point-select bench mirroring synapse-vs-mysql-honest.md harness.
Usage: python3 oltp_repro.py <port> <threads> <queries_per_thread>
"""
import sys, time, threading, statistics
import pymysql

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 13312
THREADS = int(sys.argv[2]) if len(sys.argv) > 2 else 8
QPT = int(sys.argv[3]) if len(sys.argv) > 3 else 2000

def worker(latencies, qpt):
    conn = pymysql.connect(host="127.0.0.1", port=PORT, user="root",
                           password="synapse", database="wordpress",
                           autocommit=True)
    cur = conn.cursor()
    out = []
    for i in range(qpt):
        rid = (i % 10000) + 1
        t0 = time.perf_counter()
        cur.execute(f"SELECT * FROM sbtest1 WHERE id={rid}")
        cur.fetchall()
        out.append(time.perf_counter() - t0)
    latencies.extend(out)
    conn.close()

# warmup
warm = []
worker(warm, 50)

all_lat = []
threads = []
t_start = time.perf_counter()
for _ in range(THREADS):
    lat = []
    all_lat.append(lat)
    th = threading.Thread(target=worker, args=(lat, QPT))
    th.start()
    threads.append(th)
for th in threads:
    th.join()
elapsed = time.perf_counter() - t_start

flat = [x for sub in all_lat for x in sub]
flat.sort()
total = len(flat)
ops = total / elapsed
p50 = flat[total // 2] * 1000
p95 = flat[int(total * 0.95)] * 1000
print(f"port={PORT} threads={THREADS} queries={total} elapsed={elapsed:.2f}s OPS={ops:.0f} p50={p50:.3f}ms p95={p95:.3f}ms")
