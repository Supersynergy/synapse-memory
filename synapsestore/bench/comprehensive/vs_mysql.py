#!/usr/bin/env python3
"""
Synapse vs MySQL vector-workload benchmark
Contenders:
  A) MySQL (BLOB + app-side cosine)  -- baseline
  B) synapse-turbo.py (:9477)        -- HTTP /vec
  C) synapse-ultra (:9478)           -- if alive
  (synapse-mysql-async skipped if port 3306 not answering vec queries)
Phases: Insert, Vector kNN, Hybrid
"""
import time, struct, json, random, statistics, sys
import numpy as np
import requests
import pymysql

# ── config ────────────────────────────────────────────────────────────────────
N_DOCS      = 10_000
DIM         = 384
N_QUERIES   = 1_000
TOP_K       = 10
MYSQL_HOST  = "127.0.0.1"
MYSQL_PORT  = 3306
MYSQL_USER  = "root"
MYSQL_DB    = "synapse_bench"
TURBO_URL   = "http://localhost:9477"
ULTRA_URL   = "http://localhost:9478"
CATEGORIES  = ["ai", "data", "web", "rust", "devops"]

rng = np.random.default_rng(42)

def rand_vec():
    v = rng.standard_normal(DIM).astype(np.float32)
    return v / np.linalg.norm(v)

def vec_to_blob(v): return struct.pack(f"{DIM}f", *v)
def blob_to_vec(b): return np.array(struct.unpack(f"{DIM}f", b), dtype=np.float32)
def cosine(a, b): return float(np.dot(a, b))

# ── dataset ───────────────────────────────────────────────────────────────────
print("Generating dataset…")
docs = []
for i in range(N_DOCS):
    vec = rand_vec()
    docs.append({
        "id": i,
        "title": f"doc_{i}",
        "body": f"Synapse bench document number {i} category {CATEGORIES[i % len(CATEGORIES)]} keyword_{i % 500}",
        "category": CATEGORIES[i % len(CATEGORIES)],
        "year": 2020 + (i % 6),
        "score": round(random.random(), 4),
        "vec": vec,
        "blob": vec_to_blob(vec),
    })
queries = [rand_vec() for _ in range(N_QUERIES)]
query_cats = [CATEGORIES[i % len(CATEGORIES)] for i in range(N_QUERIES)]
print(f"  {N_DOCS} docs, {N_QUERIES} queries, dim={DIM}")

results = {}

# ════════════════════════════════════════════════════════════════════════════
# CONTENDER A — MySQL blob + app-side cosine
# ════════════════════════════════════════════════════════════════════════════
print("\n[A] MySQL blob+cosine …")
try:
    conn = pymysql.connect(host=MYSQL_HOST, port=MYSQL_PORT, user=MYSQL_USER,
                           charset="utf8mb4", connect_timeout=5)
    cur = conn.cursor()
    cur.execute(f"DROP DATABASE IF EXISTS {MYSQL_DB}")
    cur.execute(f"CREATE DATABASE {MYSQL_DB}")
    cur.execute(f"USE {MYSQL_DB}")
    cur.execute("""
        CREATE TABLE docs (
            id INT PRIMARY KEY,
            title VARCHAR(64),
            body TEXT,
            category VARCHAR(16),
            year SMALLINT,
            score FLOAT,
            vec BLOB
        )
    """)
    cur.execute("CREATE INDEX idx_cat_year ON docs(category, year)")
    conn.commit()

    # ── Insert ────────────────────────────────────────────────────────────────
    BATCH = 500
    t0 = time.perf_counter()
    for i in range(0, N_DOCS, BATCH):
        batch = docs[i:i+BATCH]
        cur.executemany(
            "INSERT INTO docs(id,title,body,category,year,score,vec) VALUES(%s,%s,%s,%s,%s,%s,%s)",
            [(d["id"], d["title"], d["body"], d["category"], d["year"], d["score"], d["blob"]) for d in batch]
        )
        conn.commit()
    insert_time = time.perf_counter() - t0
    mysql_insert_ops = N_DOCS / insert_time

    # ── Vec kNN (full scan) ───────────────────────────────────────────────────
    latencies_knn = []
    for q in queries:
        t0 = time.perf_counter()
        cur.execute("SELECT id, vec FROM docs")
        rows = cur.fetchall()
        scored = [(cosine(q, blob_to_vec(r[1])), r[0]) for r in rows]
        scored.sort(reverse=True)
        top = scored[:TOP_K]
        latencies_knn.append((time.perf_counter() - t0) * 1000)
    mysql_knn_ops = N_QUERIES / (sum(latencies_knn) / 1000)
    mysql_knn_p50 = statistics.median(latencies_knn)
    mysql_knn_p99 = sorted(latencies_knn)[int(0.99 * N_QUERIES)]

    # ── Hybrid (filter by category, then cosine on subset) ────────────────────
    latencies_hybrid = []
    for q, cat in zip(queries, query_cats):
        t0 = time.perf_counter()
        cur.execute("SELECT id, vec FROM docs WHERE category=%s", (cat,))
        rows = cur.fetchall()
        scored = [(cosine(q, blob_to_vec(r[1])), r[0]) for r in rows]
        scored.sort(reverse=True)
        top = scored[:TOP_K]
        latencies_hybrid.append((time.perf_counter() - t0) * 1000)
    mysql_hybrid_ops = N_QUERIES / (sum(latencies_hybrid) / 1000)
    mysql_hybrid_p50 = statistics.median(latencies_hybrid)
    mysql_hybrid_p99 = sorted(latencies_hybrid)[int(0.99 * N_QUERIES)]

    results["mysql"] = {
        "insert_ops": mysql_insert_ops,
        "knn_ops": mysql_knn_ops, "knn_p50": mysql_knn_p50, "knn_p99": mysql_knn_p99,
        "hybrid_ops": mysql_hybrid_ops, "hybrid_p50": mysql_hybrid_p50, "hybrid_p99": mysql_hybrid_p99,
    }
    print(f"  insert {mysql_insert_ops:.0f} ops/s  kNN {mysql_knn_ops:.2f} ops/s  p50={mysql_knn_p50:.1f}ms")
    conn.close()
except Exception as e:
    print(f"  SKIPPED: {e}")
    results["mysql"] = None

# ════════════════════════════════════════════════════════════════════════════
# CONTENDER B — synapse-turbo.py :9477 /vec
# ════════════════════════════════════════════════════════════════════════════
print("\n[B] synapse-turbo (:9477) …")
try:
    r = requests.get(f"{TURBO_URL}/health", timeout=2)
    r.raise_for_status()

    # Insert: turbo is read-only cache over existing brain.db — measure query throughput
    # For insert we use /put if available, else skip insert phase for turbo
    # Try /put
    put_ok = False
    try:
        r2 = requests.post(f"{TURBO_URL}/put", json={"title": "bench_test", "text": "bench"}, timeout=2)
        put_ok = r2.status_code == 200
    except:
        pass

    if put_ok:
        t0 = time.perf_counter()
        for d in docs[:1000]:  # only 1k for insert test (turbo is cache, not primary store)
            requests.post(f"{TURBO_URL}/put", json={"title": d["title"], "text": d["body"]}, timeout=5)
        turbo_insert_ops = 1000 / (time.perf_counter() - t0)
    else:
        turbo_insert_ops = None  # read-only

    # Vec kNN (actual vec search via embed)
    latencies_knn = []
    for i, q in enumerate(queries[:N_QUERIES]):
        q_text = f"keyword_{i % 500} category {query_cats[i]}"
        t0 = time.perf_counter()
        r = requests.get(f"{TURBO_URL}/vec", params={"q": q_text, "limit": TOP_K}, timeout=10)
        latencies_knn.append((time.perf_counter() - t0) * 1000)
    turbo_knn_ops = N_QUERIES / (sum(latencies_knn) / 1000)
    turbo_knn_p50 = statistics.median(latencies_knn)
    turbo_knn_p99 = sorted(latencies_knn)[int(0.99 * N_QUERIES)]

    # Hybrid
    latencies_hybrid = []
    for i, (q, cat) in enumerate(zip(queries[:N_QUERIES], query_cats)):
        q_text = f"keyword_{i % 500} {cat}"
        t0 = time.perf_counter()
        r = requests.get(f"{TURBO_URL}/hybrid", params={"q": q_text, "limit": TOP_K}, timeout=10)
        latencies_hybrid.append((time.perf_counter() - t0) * 1000)
    turbo_hybrid_ops = N_QUERIES / (sum(latencies_hybrid) / 1000)
    turbo_hybrid_p50 = statistics.median(latencies_hybrid)
    turbo_hybrid_p99 = sorted(latencies_hybrid)[int(0.99 * N_QUERIES)]

    results["turbo"] = {
        "insert_ops": turbo_insert_ops,
        "knn_ops": turbo_knn_ops, "knn_p50": turbo_knn_p50, "knn_p99": turbo_knn_p99,
        "hybrid_ops": turbo_hybrid_ops, "hybrid_p50": turbo_hybrid_p50, "hybrid_p99": turbo_hybrid_p99,
    }
    print(f"  kNN {turbo_knn_ops:.1f} ops/s  p50={turbo_knn_p50:.2f}ms  p99={turbo_knn_p99:.2f}ms")
except Exception as e:
    print(f"  SKIPPED: {e}")
    results["turbo"] = None

# ════════════════════════════════════════════════════════════════════════════
# CONTENDER C — synapse-ultra :9478
# ════════════════════════════════════════════════════════════════════════════
print("\n[C] synapse-ultra (:9478) …")
try:
    r = requests.get(f"{ULTRA_URL}/health", timeout=2)
    r.raise_for_status()

    latencies_knn = []
    for i, q in enumerate(queries[:N_QUERIES]):
        q_text = f"keyword_{i % 500} category {query_cats[i]}"
        t0 = time.perf_counter()
        r = requests.get(f"{ULTRA_URL}/vec", params={"q": q_text, "limit": TOP_K}, timeout=10)
        latencies_knn.append((time.perf_counter() - t0) * 1000)
    ultra_knn_ops = N_QUERIES / (sum(latencies_knn) / 1000)
    ultra_knn_p50 = statistics.median(latencies_knn)
    ultra_knn_p99 = sorted(latencies_knn)[int(0.99 * N_QUERIES)]

    latencies_hybrid = []
    for i, (q, cat) in enumerate(zip(queries[:N_QUERIES], query_cats)):
        q_text = f"keyword_{i % 500} {cat}"
        t0 = time.perf_counter()
        r = requests.get(f"{ULTRA_URL}/hybrid", params={"q": q_text, "limit": TOP_K}, timeout=10)
        latencies_hybrid.append((time.perf_counter() - t0) * 1000)
    ultra_hybrid_ops = N_QUERIES / (sum(latencies_hybrid) / 1000)
    ultra_hybrid_p50 = statistics.median(latencies_hybrid)
    ultra_hybrid_p99 = sorted(latencies_hybrid)[int(0.99 * N_QUERIES)]

    results["ultra"] = {
        "knn_ops": ultra_knn_ops, "knn_p50": ultra_knn_p50, "knn_p99": ultra_knn_p99,
        "hybrid_ops": ultra_hybrid_ops, "hybrid_p50": ultra_hybrid_p50, "hybrid_p99": ultra_hybrid_p99,
    }
    print(f"  kNN {ultra_knn_ops:.1f} ops/s  p50={ultra_knn_p50:.2f}ms  p99={ultra_knn_p99:.2f}ms")
except Exception as e:
    print(f"  SKIPPED: {e}")
    results["ultra"] = None

# ════════════════════════════════════════════════════════════════════════════
# SAVE RESULTS JSON
# ════════════════════════════════════════════════════════════════════════════
import pathlib
out = pathlib.Path(__file__).parent / "vs_mysql_results.json"
with open(out, "w") as f:
    json.dump(results, f, indent=2, default=str)
print(f"\nResults saved → {out}")
print("Done.")
