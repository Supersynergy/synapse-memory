#!/usr/bin/env python3
"""
Step 1: Extract ground truth for industry benchmark.

Two outputs:
  ground_truth.bin     - msgpack: raw vectors for ANN engines (usearch, lance, sqlite-vec)
  query_texts.json     - 1000 doc texts (for ultra HTTP text-embedding path)

Ground truth = brute-force cosine top-100 on full 168k corpus.
"""
import os, random, time, sqlite3, json
import sqlite_vec
import msgpack
import numpy as np
from pathlib import Path

BRAIN_DB = Path(os.environ.get("BRAIN_DB", "~/.synapse/brain.db")).expanduser()
OUT_BIN = Path(__file__).parent / "ground_truth.bin"
OUT_TEXTS = Path(__file__).parent / "query_texts.json"
N_QUERIES = 1000
GT_K = 100
SEED = 42

print(f"[01] Loading vectors from {BRAIN_DB}...")
t0 = time.perf_counter()

db = sqlite3.connect(str(BRAIN_DB))
db.enable_load_extension(True)
sqlite_vec.load(db)
db.enable_load_extension(False)

# Load all vectors
rows = db.execute("SELECT id, embedding FROM docs_vec ORDER BY id").fetchall()
print(f"  {len(rows)} vectors in {time.perf_counter()-t0:.2f}s")

doc_ids = np.array([r[0] for r in rows], dtype=np.int64)
vecs = np.zeros((len(rows), 384), dtype=np.float32)
for i, (_, blob) in enumerate(rows):
    vecs[i] = np.frombuffer(blob, dtype=np.float32)

# Normalize for cosine similarity
norms = np.linalg.norm(vecs, axis=1, keepdims=True)
norms[norms == 0] = 1.0
vecs_n = (vecs / norms).astype(np.float32)

N = len(rows)
rng = random.Random(SEED)
query_indices = sorted(rng.sample(range(N), min(N_QUERIES, N)))

# Load corresponding texts for ultra
print("  Loading query texts from docs table...")
qid_to_idx = {int(doc_ids[i]): i for i in query_indices}
placeholders = ",".join("?" * len(query_indices))
query_doc_ids = [int(doc_ids[i]) for i in query_indices]
text_rows = db.execute(
    f"SELECT id, COALESCE(title, substr(text,1,200)) FROM docs WHERE id IN ({placeholders})",
    query_doc_ids
).fetchall()
text_map = {r[0]: r[1] for r in text_rows}
query_texts = [text_map.get(qid, f"doc_{qid}") for qid in query_doc_ids]

# Brute-force ground truth
print(f"  Computing brute-force top-{GT_K} for {len(query_indices)} queries...")
t1 = time.perf_counter()

q_vecs = vecs_n[query_indices]  # (N_q, 384)
gt_ids = []
# Batch dot: process in chunks
CHUNK = 50
for qi in range(len(query_indices)):
    qv = q_vecs[qi]
    scores = vecs_n @ qv  # cosine dot on normalized vecs
    top_idx = np.argpartition(scores, -GT_K)[-GT_K:]
    top_idx = top_idx[np.argsort(scores[top_idx])[::-1]]
    gt_ids.append([int(doc_ids[j]) for j in top_idx])

elapsed = time.perf_counter() - t1
print(f"  Brute-force done: {elapsed:.2f}s  ({elapsed/len(query_indices)*1000:.1f}ms/query avg)")

# Save binary ground truth
payload = {
    "n_queries": len(query_indices),
    "n_corpus": N,
    "dims": 384,
    "gt_k": GT_K,
    "corpus_ids": doc_ids.tolist(),
    "corpus_vecs_shape": [N, 384],
    "query_doc_ids": query_doc_ids,
    "query_vecs": [q_vecs[i].tolist() for i in range(len(query_indices))],
    "gt_ids": gt_ids,
}
OUT_BIN.write_bytes(msgpack.packb(payload, use_bin_type=True))
print(f"  → {OUT_BIN}  ({OUT_BIN.stat().st_size/1e6:.1f} MB)")

# Save texts
OUT_TEXTS.write_text(json.dumps({
    "n_queries": len(query_indices),
    "query_doc_ids": query_doc_ids,
    "query_texts": query_texts,
}, indent=2))
print(f"  → {OUT_TEXTS}")

# Also save corpus vecs as a numpy file for other engines
corpus_npy = Path(__file__).parent / "corpus_vecs.npy"
np.save(corpus_npy, vecs)  # raw (un-normalized) for engines that normalize internally
corpus_ids_npy = Path(__file__).parent / "corpus_ids.npy"
np.save(corpus_ids_npy, doc_ids)
print(f"  → {corpus_npy}  {corpus_ids_npy}")

print("[01] Done.")
