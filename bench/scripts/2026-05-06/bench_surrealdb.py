#!/usr/bin/env python3.13
"""Quick SurrealDB bench: 50k x 384d vectors, same workload as 10-way."""
import time, numpy as np
from surrealdb import Surreal

N, D, K, Q = 5000, 384, 10, 20  # smaller corpus for SurrealDB speed
np.random.seed(42)
X = np.random.randn(N, D).astype(np.float32)
X /= np.linalg.norm(X, axis=1, keepdims=True)
queries = np.random.randn(Q, D).astype(np.float32)
queries /= np.linalg.norm(queries, axis=1, keepdims=True)

# Ground truth
gt = np.argsort(-(queries @ X.T), axis=1)[:, :K]

db = Surreal("http://127.0.0.1:8000/rpc")
db.signin({"username": "root", "password": "root"})
db.use("test", "test")

# Build
print(f"=== SurrealDB v2 bench ({N} x {D}d) ===")
try: db.query("REMOVE TABLE doc;")
except: pass
db.query("DEFINE TABLE doc SCHEMAFULL;")
db.query("DEFINE FIELD vec ON doc TYPE array<float>;")

t0 = time.time()
for i in range(N):
    db.query(f"CREATE doc:{i} SET vec = {X[i].tolist()};")
build_t = time.time() - t0
print(f"build {N} docs: {build_t:.2f}s ({N/build_t:.0f} ins/s)")

# Define HNSW index (SurrealDB v2 supports this)
try:
    db.query(f"DEFINE INDEX vec_hnsw ON doc FIELDS vec HNSW DIMENSION {D} M 32 EFC 200;")
    print("HNSW index created")
except Exception as e:
    print(f"HNSW err: {e}")

# Query
t0 = time.time()
hits = []
for q in queries:
    r = db.query(f"SELECT id FROM doc WHERE vec <|{K},EUCLIDEAN|> {q.tolist()};")
    if r and isinstance(r, list):
        ids = [int(str(h.get('id','0')).rsplit(':',1)[-1]) for h in (r[0].get('result') or r) if isinstance(h, dict)]
        hits.append(ids)
    else:
        hits.append([])
q_t = (time.time() - t0) / Q
recall = sum(len(set(hits[i]) & set(gt[i].tolist())) for i in range(Q)) / (Q * K)

print(f"query p50 ms: {q_t*1000:.2f}")
print(f"QPS:          {1/q_t:.0f}")
print(f"recall@{K}:    {recall:.3f}")
