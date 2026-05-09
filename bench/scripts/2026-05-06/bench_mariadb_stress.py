#!/usr/bin/env python3.13
"""Stress bench: Synapse vs MariaDB 11.7 vector + concurrent load."""
import sys, time, threading
sys.path.insert(0, '/Users/master/.local/lib')
import synxlib
from concurrent.futures import ThreadPoolExecutor

import numpy as np
N, D, K, Q = 5000, 384, 10, 50
np.random.seed(42)
X = np.random.randn(N, D).astype(np.float32)
X /= np.linalg.norm(X, axis=1, keepdims=True)
queries = np.random.randn(Q, D).astype(np.float32)
queries /= np.linalg.norm(queries, axis=1, keepdims=True)

results = {}
def bench(name, fn, runs=3):
    times=[]
    for _ in range(runs):
        try:
            t0=time.time(); fn(); times.append((time.time()-t0)*1000)
        except Exception as e:
            print(f'  {name}: ERR {str(e)[:80]}'); return
    results[name] = min(times)
    print(f'  {name:42s} min={min(times):8.2f}ms')

def concurrent(name, fn, n_workers=8, n_calls_per=50):
    t0=time.time()
    with ThreadPoolExecutor(max_workers=n_workers) as ex:
        list(ex.map(lambda _: [fn() for _ in range(n_calls_per)], range(n_workers)))
    total = (time.time()-t0)*1000
    ops = n_workers * n_calls_per
    results[name] = total
    print(f'  {name:42s} {total:8.2f}ms total = {ops/total*1000:.0f} ops/s')

# === SYNAPSE ===
print("=== Synapse (live) ===")
# Auth
synxlib.call({'op':'Auth','args':{'token':'test123'}})
bench('Synapse ping ×1000', lambda: [synxlib.ping() for _ in range(1000)])
bench('Synapse fts_direct ×100', lambda: [synxlib.fts_direct('python', 5) for _ in range(100)])
bench('Synapse Sql COUNT ×100', lambda: [synxlib.sql('SELECT COUNT(*) FROM docs') for _ in range(100)])
bench('Synapse hybrid_cached ×30', lambda: [synxlib.hybrid_cached('vector test', 5) for _ in range(30)])

# === MARIADB ===
print("\n=== MariaDB 11.7 (Docker) ===")
import pymysql
conn = pymysql.connect(host='127.0.0.1', port=3307, user='root', password='test', database='bench')
c = conn.cursor()
c.execute("DROP TABLE IF EXISTS docs")
c.execute(f"CREATE TABLE docs (id INT PRIMARY KEY, embedding VECTOR({D}) NOT NULL, VECTOR INDEX vidx (embedding) DISTANCE=cosine)")
print(f'  table created with VECTOR({D}) index')

# Insert
def to_vec_blob(v):
    return v.astype('<f4').tobytes()
t0=time.time()
for i in range(N):
    c.execute("INSERT INTO docs (id, embedding) VALUES (%s, VEC_FromText(%s))",
              (i, '[' + ','.join(f'{x:.4f}' for x in X[i]) + ']'))
    if (i+1) % 500 == 0: conn.commit()
conn.commit()
build_t = time.time()-t0
print(f'  build {N}: {build_t:.1f}s = {N/build_t:.0f} ins/s')

# Single-thread vec query
def maria_vec():
    qs = '[' + ','.join(f'{x:.4f}' for x in queries[0]) + ']'
    c.execute(f"SELECT id FROM docs ORDER BY VEC_DISTANCE_COSINE(embedding, VEC_FromText(%s)) LIMIT %s", (qs, K))
    c.fetchall()
bench('MariaDB vec query ×30', lambda: [maria_vec() for _ in range(30)])

# Stress: 8 concurrent workers × 25 vec queries each
print("\n=== STRESS — 8 workers × 25 ops ===")
def synapse_w():
    for _ in range(25): synxlib.search('vector', mode='Lex', limit=5, embed_query=False)
def maria_vec_w():
    cn = pymysql.connect(host='127.0.0.1', port=3307, user='root', password='test', database='bench')
    cur = cn.cursor()
    qs = '[' + ','.join(f'{x:.4f}' for x in queries[0]) + ']'
    for _ in range(25):
        cur.execute(f"SELECT id FROM docs ORDER BY VEC_DISTANCE_COSINE(embedding, VEC_FromText(%s)) LIMIT %s", (qs, K))
        cur.fetchall()
    cn.close()

concurrent('Synapse 8w × 25 Lex (200 ops)', synapse_w, 8, 1)
concurrent('MariaDB 8w × 25 vec (200 ops)', maria_vec_w, 8, 1)

# Synapse direct (no daemon) under stress
def syn_direct_w():
    for _ in range(25): synxlib.fts_direct('python', 5)
concurrent('Synapse 8w × 25 fts_direct', syn_direct_w, 8, 1)

print("\n=== RANKED ===")
for k, v in sorted(results.items(), key=lambda x: x[1]):
    print(f'  {v:8.2f}ms  {k}')

c.close(); conn.close()
