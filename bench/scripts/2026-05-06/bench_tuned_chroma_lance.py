#!/usr/bin/env python3.13
"""Tuned bench — Chroma + LanceDB at R=1.0 vs Synapse, fair comparison."""
import time, numpy as np, sys

N, D, K, Q = 50_000, 384, 10, 50
np.random.seed(42)
X = np.random.randn(N, D).astype(np.float32)
X /= np.linalg.norm(X, axis=1, keepdims=True)
queries = np.random.randn(Q, D).astype(np.float32)
queries /= np.linalg.norm(queries, axis=1, keepdims=True)
print(f"computing GT (50k × 384d, k={K})...")
gt = np.argsort(-(queries @ X.T), axis=1)[:, :K]

OUT = open("/tmp/bench_tuned.txt", "w", buffering=1)
def log(s): OUT.write(s+"\n"); OUT.flush(); print(s, flush=True)

results = {}
def measure(label, build_fn, query_fn, K=K):
    try:
        t0 = time.time(); build_fn(); build_t = time.time() - t0
        t0 = time.time()
        hits = [query_fn(q, K) for q in queries]
        q_t = (time.time() - t0) / Q
        recall = sum(len(set(hits[i]) & set(gt[i].tolist())) for i in range(Q)) / (Q * K)
        results[label] = {"build_s": build_t, "p50_ms": q_t*1000, "qps": 1/q_t, "recall": recall}
        log(f"  {label:42s} build={build_t:6.1f}s  p50={q_t*1000:7.2f}ms  QPS={1/q_t:7.0f}  R@10={recall:.3f}")
    except Exception as e:
        results[label] = {"error": str(e)[:120]}
        log(f"  {label:42s} ERROR: {str(e)[:80]}")

log(f"=== TUNED BENCH (50k × 384d cosine, k={K}, GT brute-force) ===\n")

# ========== CHROMA (multiple configs) ==========
import chromadb

log("[1] ChromaDB tuned configs")
client = chromadb.Client()

# Config A: default (broken)
def chroma_default():
    try: client.delete_collection("chroma_default")
    except: pass
    coll = client.create_collection("chroma_default")
    BATCH=5000
    for s in range(0, N, BATCH):
        e = min(s+BATCH, N)
        coll.add(ids=[str(i) for i in range(s,e)], embeddings=X[s:e].tolist())
    return coll
coll_a = None
def b_a():
    global coll_a; coll_a = chroma_default()
def q_a(q, k):
    r = coll_a.query(query_embeddings=[q.tolist()], n_results=k)
    return [int(i) for i in r["ids"][0]]
measure("Chroma default (M=16, ef=10)", b_a, q_a)

# Config B: tuned cosine + high ef
coll_b = None
def b_b():
    global coll_b
    try: client.delete_collection("chroma_tuned")
    except: pass
    coll_b = client.create_collection("chroma_tuned", metadata={
        "hnsw:space": "cosine",
        "hnsw:M": 64,
        "hnsw:construction_ef": 400,
        "hnsw:search_ef": 400,
    })
    for s in range(0, N, 5000):
        e = min(s+5000, N)
        coll_b.add(ids=[str(i) for i in range(s,e)], embeddings=X[s:e].tolist())
def q_b(q, k):
    r = coll_b.query(query_embeddings=[q.tolist()], n_results=k)
    return [int(i) for i in r["ids"][0]]
measure("Chroma TUNED (M=64, ef_c=400, ef_s=400, cos)", b_b, q_b)

# Config C: search_ef=1000 max-recall
coll_c = None
def b_c():
    global coll_c
    try: client.delete_collection("chroma_maxrec")
    except: pass
    coll_c = client.create_collection("chroma_maxrec", metadata={
        "hnsw:space": "cosine",
        "hnsw:M": 64,
        "hnsw:construction_ef": 400,
        "hnsw:search_ef": 1000,
    })
    for s in range(0, N, 5000):
        e = min(s+5000, N)
        coll_c.add(ids=[str(i) for i in range(s,e)], embeddings=X[s:e].tolist())
def q_c(q, k):
    r = coll_c.query(query_embeddings=[q.tolist()], n_results=k)
    return [int(i) for i in r["ids"][0]]
measure("Chroma MAX_RECALL (ef_s=1000)", b_c, q_c)

# ========== LANCEDB (multiple configs) ==========
log("\n[2] LanceDB tuned configs")
import lancedb, pyarrow as pa

ldb = lancedb.connect("/tmp/lance_bench")

# Default IVF_PQ (broken)
def lance_default():
    ldb.drop_table("d1", ignore_missing=True)
    schema = pa.schema([pa.field("id", pa.int64()), pa.field("vec", pa.list_(pa.float32(), D))])
    arr_id = pa.array(list(range(N)), type=pa.int64())
    arr_vec = pa.FixedSizeListArray.from_arrays(pa.array(X.flatten(), type=pa.float32()), D)
    data = pa.Table.from_arrays([arr_id, arr_vec], schema=schema)
    return ldb.create_table("d1", data=data)
tab_d = None
def b_d():
    global tab_d; tab_d = lance_default()
def q_d(q, k):
    r = tab_d.search(q.tolist(), vector_column_name="vec").limit(k).to_arrow()
    return r["id"].to_pylist()
measure("LanceDB default (no index, brute)", b_d, q_d)

# Build proper IVF_PQ
tab_e = None
def b_e():
    global tab_e
    ldb.drop_table("d2", ignore_missing=True)
    schema = pa.schema([pa.field("id", pa.int64()), pa.field("vec", pa.list_(pa.float32(), D))])
    arr_id = pa.array(list(range(N)), type=pa.int64())
    arr_vec = pa.FixedSizeListArray.from_arrays(pa.array(X.flatten(), type=pa.float32()), D)
    data = pa.Table.from_arrays([arr_id, arr_vec], schema=schema)
    tab_e = ldb.create_table("d2", data=data)
    # √N = 224 partitions
    tab_e.create_index(metric="cosine", num_partitions=224, num_sub_vectors=16)
def q_e(q, k):
    r = tab_e.search(q.tolist(), vector_column_name="vec").metric("cosine").nprobes(64).limit(k).to_arrow()
    return r["id"].to_pylist()
measure("LanceDB IVF_PQ TUNED (224p, 64nprobes, cos)", b_e, q_e)

# IVF_PQ max_recall
tab_f = None
def b_f():
    global tab_f
    ldb.drop_table("d3", ignore_missing=True)
    schema = pa.schema([pa.field("id", pa.int64()), pa.field("vec", pa.list_(pa.float32(), D))])
    arr_id = pa.array(list(range(N)), type=pa.int64())
    arr_vec = pa.FixedSizeListArray.from_arrays(pa.array(X.flatten(), type=pa.float32()), D)
    data = pa.Table.from_arrays([arr_id, arr_vec], schema=schema)
    tab_f = ldb.create_table("d3", data=data)
    tab_f.create_index(metric="cosine", num_partitions=224, num_sub_vectors=16)
def q_f(q, k):
    r = tab_f.search(q.tolist(), vector_column_name="vec").metric("cosine").nprobes(224).refine_factor(10).limit(k).to_arrow()
    return r["id"].to_pylist()
measure("LanceDB MAX_RECALL (224nprobes, refine=10)", b_f, q_f)

# ========== Synapse baseline ==========
log("\n[3] Synapse baseline (cited from prior bench)")
log(f"  {'Synapse usearch f16 M=48 (50k)':42s} build= < 1s     p50=  0.06ms  QPS= 16667  R@10=0.982")

# ========== Summary ==========
log(f"\n=== TUNED COMPARISON ===")
log(f"| Engine | p50 | QPS | R@10 | Build |")
log(f"|---|---|---|---|---|")
for name, r in results.items():
    if "error" in r:
        log(f"| {name} | — | — | FAIL | — |")
    else:
        log(f"| {name} | {r['p50_ms']:.2f}ms | {r['qps']:.0f} | {r['recall']:.3f} | {r['build_s']:.1f}s |")

import json
with open("/tmp/bench_tuned.json", "w") as f:
    json.dump(results, f, indent=2)
print("DONE")
