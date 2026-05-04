#!/usr/bin/env python3
"""Generate dataset.parquet for comprehensive bench."""
import os, hashlib, struct
import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq

DIR = os.path.dirname(os.path.abspath(__file__))
CATEGORIES = ["tech", "science", "news", "docs", "forum", "blog", "paper", "wiki"]
N = int(os.environ.get("N", 100_000))


def sha_vec(seed: str, dim=384) -> bytes:
    h = hashlib.sha256(seed.encode()).digest()
    floats = []
    for i in range(dim):
        b = h[(i * 4) % 32: (i * 4) % 32 + 4]
        floats.append(struct.unpack("<f", b)[0])
    arr = np.array(floats, dtype=np.float32)
    arr /= np.linalg.norm(arr) + 1e-9
    return arr.tobytes()


ids, texts, vecs, cats, scores, timestamps, sources, langs = [], [], [], [], [], [], [], []
rng = np.random.default_rng(42)

for i in range(N):
    doc_id = f"doc_{i:07d}"
    text = f"Document {i}: topic {i % 1000} cluster {i % 37} embedding bench sample"
    ids.append(doc_id)
    texts.append(text)
    vecs.append(sha_vec(doc_id))
    cats.append(CATEGORIES[i % len(CATEGORIES)])
    scores.append(float(rng.uniform(0, 1)))
    timestamps.append(1700000000 + i * 60)
    sources.append(f"source_{i % 20}")
    langs.append("en" if i % 10 != 0 else "de")

schema = pa.schema([
    pa.field("id", pa.string()),
    pa.field("text", pa.string()),
    pa.field("vec", pa.binary()),
    pa.field("category", pa.string()),
    pa.field("score", pa.float64()),
    pa.field("timestamp", pa.int64()),
    pa.field("source", pa.string()),
    pa.field("lang", pa.string()),
])

tbl = pa.table({
    "id": ids, "text": texts, "vec": vecs, "category": cats,
    "score": scores, "timestamp": timestamps, "source": sources, "lang": langs,
}, schema=schema)

out = os.path.join(DIR, "dataset.parquet")
pq.write_table(tbl, out, compression="snappy")
print(f"Written {N} rows -> {out} ({os.path.getsize(out)/1e6:.1f} MB)")
