#!/usr/bin/env bash
# setup.sh — creates venv, installs deps, generates 100k synthetic docs to dataset.parquet
set -euo pipefail

VENV="$HOME/.venvs/synapse-bench"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATASET="$DIR/dataset.parquet"
N_DOCS="${1:-100000}"

echo "[setup] Python venv at $VENV"
if [ ! -d "$VENV" ]; then
  python3.12 -m venv "$VENV"
fi

source "$VENV/bin/activate"
pip install -q --upgrade pip

echo "[setup] Installing deps..."
pip install -q \
  pyarrow pandas numpy psutil \
  lancedb \
  "chromadb>=0.5" \
  duckdb \
  sentence-transformers \
  requests

# sqlite-vec wheel (arm64 py3.12)
pip install -q sqlite-vec || echo "[warn] sqlite-vec install failed, will try at runtime"

echo "[setup] Deps installed. Generating $N_DOCS synthetic docs..."

python3 - <<'PYEOF'
import os, sys, random, string, struct, time
import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq

n = int(os.environ.get("N_DOCS", sys.argv[1] if len(sys.argv) > 1 else "100000"))
dataset_path = os.environ.get("DATASET_PATH", os.path.join(os.path.dirname(os.path.abspath(__file__)), "dataset.parquet"))

if os.path.exists(dataset_path):
    tbl = pq.read_table(dataset_path)
    if len(tbl) >= n:
        print(f"[setup] dataset.parquet already has {len(tbl)} docs, skipping generation")
        sys.exit(0)

print(f"[setup] Generating {n} docs with 384-dim embeddings...")

# Use random normalized vecs (avoids BGE CPU bottleneck for setup validation)
# Real workload uses precomputed vecs — same fairness across all adapters
rng = np.random.default_rng(42)

words = ["search", "memory", "vector", "database", "benchmark", "performance",
         "retrieval", "embedding", "latency", "throughput", "concurrent", "index",
         "neural", "semantic", "query", "storage", "cache", "hybrid", "full-text",
         "metadata", "filter", "similarity", "cosine", "euclidean", "insert"]

def gen_text(min_len=200, max_len=2000):
    target = random.randint(min_len, max_len)
    parts = []
    while sum(len(p) for p in parts) < target:
        parts.append(" ".join(random.choices(words, k=random.randint(5, 20))))
    return " ".join(parts)[:max_len]

batch_size = 10000
all_ids, all_texts, all_vecs, all_cat, all_score, all_ts, all_src, all_lang = [], [], [], [], [], [], [], []

categories = ["tech", "science", "news", "docs", "forum", "blog", "paper", "wiki"]
sources = ["web", "api", "file", "stream", "manual"]
langs = ["en", "de", "fr", "es", "it"]

t0 = time.time()
for i in range(n):
    all_ids.append(f"doc_{i:07d}")
    all_texts.append(gen_text())
    all_cat.append(random.choice(categories))
    all_score.append(round(random.uniform(0.0, 1.0), 4))
    all_ts.append(int(time.time()) - random.randint(0, 86400 * 365))
    all_src.append(random.choice(sources))
    all_lang.append(random.choice(langs))
    if (i + 1) % batch_size == 0:
        elapsed = time.time() - t0
        print(f"  {i+1}/{n} docs generated ({elapsed:.1f}s)", flush=True)

print("[setup] Generating random 384-dim unit vectors...")
vecs_np = rng.standard_normal((n, 384)).astype(np.float32)
norms = np.linalg.norm(vecs_np, axis=1, keepdims=True)
vecs_np /= norms
vecs_list = [vecs_np[i].tobytes() for i in range(n)]

table = pa.table({
    "id": all_ids,
    "text": all_texts,
    "vec": pa.array(vecs_list, type=pa.binary()),
    "category": all_cat,
    "score": all_score,
    "timestamp": all_ts,
    "source": all_src,
    "lang": all_lang,
})

pq.write_table(table, dataset_path, compression="snappy")
size_mb = os.path.getsize(dataset_path) / 1e6
print(f"[setup] dataset.parquet written: {n} docs, {size_mb:.1f} MB, {time.time()-t0:.1f}s")
PYEOF

echo "[setup] Done. Dataset: $DATASET"
echo "[setup] Activate venv: source $VENV/bin/activate"
