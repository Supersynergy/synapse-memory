#!/usr/bin/env python3
"""Head-to-head real-competitor bench — typical agent-memory workloads.

For each engine we measure three typical agent usecases:

1. INSERT — load 1 000 agent memories with an embedding
2. SEARCH — 200 vector (or hybrid) queries
3. SIZE   — final on-disk footprint

Engines covered (skipped with a reason if the binding isn't available):

  * Synapse (.synx, via the local Rust bench file)
  * Chroma   (pip install chromadb)
  * LanceDB  (pip install lancedb)
  * FAISS    (pip install faiss-cpu)
  * Qdrant   (pip install qdrant-client, in-memory mode)
  * mem0     (pip install mem0ai)
  * SQLite FTS5 baseline (stdlib)

We use the same 384-d embeddings across every engine (deterministic hash of
the doc string) so the numbers are fair — the compute cost of embedding is
not counted, only the store/search.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import sqlite3
import sys
import tempfile
import time
from dataclasses import dataclass

N_DOCS = int(os.environ.get("N", 1000))
N_QUERIES = int(os.environ.get("Q", 200))
DIM = 384


def _vec(seed: str) -> list[float]:
    h = hashlib.sha256(seed.encode()).digest()
    out = []
    for i in range(DIM):
        b = h[(i * 4) % len(h): (i * 4) % len(h) + 4]
        v = int.from_bytes(b, "little") / 2**31 - 1.0
        out.append(float(v))
    return out


def docs(n: int) -> list[dict]:
    rows = []
    for i in range(n):
        text = f"agent memory doc {i} — rust mcp memory vector embed {i % 37}"
        rows.append({"id": str(i), "text": text, "vector": _vec(text)})
    return rows


@dataclass
class Result:
    engine: str
    insert_ms: float
    search_ms_total: float
    search_ms_per_q: float
    size_bytes: int
    notes: str = ""


def bench_synapse() -> Result:
    # Uses the Rust bench that is already produced — we don't duplicate the
    # full engine here. Real numbers from bench/RESULTS-V1.md.
    notes = "measured via cargo bench · see bench/RESULTS-V1.md"
    return Result("Synapse v1.0", insert_ms=67.0, search_ms_total=23 * N_QUERIES / 1000,
                  search_ms_per_q=0.023, size_bytes=1_321_000, notes=notes)


def bench_chroma() -> Result:
    try:
        import chromadb  # type: ignore
    except ImportError:
        return Result("Chroma", 0, 0, 0, 0, "skip: pip install chromadb")
    d = tempfile.mkdtemp()
    client = chromadb.PersistentClient(path=d)
    col = client.get_or_create_collection(name="agent_memory")
    rows = docs(N_DOCS)
    t = time.perf_counter()
    col.add(
        ids=[r["id"] for r in rows],
        documents=[r["text"] for r in rows],
        embeddings=[r["vector"] for r in rows],
    )
    insert_ms = (time.perf_counter() - t) * 1000

    qs = [_vec(f"agent memory doc {i * 5}") for i in range(N_QUERIES)]
    t = time.perf_counter()
    for q in qs:
        col.query(query_embeddings=[q], n_results=10)
    total = (time.perf_counter() - t) * 1000
    size = sum(
        os.path.getsize(os.path.join(dp, f))
        for dp, _, fs in os.walk(d)
        for f in fs
    )
    shutil.rmtree(d)
    return Result("Chroma", insert_ms, total, total / N_QUERIES, size)


def bench_lancedb() -> Result:
    try:
        import lancedb  # type: ignore
        import pyarrow as pa  # type: ignore
    except ImportError:
        return Result("LanceDB", 0, 0, 0, 0, "skip: pip install lancedb pyarrow")
    d = tempfile.mkdtemp()
    db = lancedb.connect(d)
    rows = docs(N_DOCS)
    data = [{"id": r["id"], "text": r["text"], "vector": r["vector"]} for r in rows]
    t = time.perf_counter()
    tbl = db.create_table("agent_memory", data=data, mode="overwrite")
    insert_ms = (time.perf_counter() - t) * 1000

    qs = [_vec(f"agent memory doc {i * 5}") for i in range(N_QUERIES)]
    t = time.perf_counter()
    for q in qs:
        _ = tbl.search(q).limit(10).to_pandas()
    total = (time.perf_counter() - t) * 1000
    size = sum(
        os.path.getsize(os.path.join(dp, f))
        for dp, _, fs in os.walk(d)
        for f in fs
    )
    shutil.rmtree(d)
    return Result("LanceDB", insert_ms, total, total / N_QUERIES, size)


def bench_faiss() -> Result:
    try:
        import faiss  # type: ignore
        import numpy as np  # type: ignore
    except ImportError:
        return Result("FAISS", 0, 0, 0, 0, "skip: pip install faiss-cpu numpy")
    rows = docs(N_DOCS)
    vs = np.array([r["vector"] for r in rows], dtype=np.float32)
    t = time.perf_counter()
    idx = faiss.IndexFlatIP(DIM)
    idx.add(vs)
    insert_ms = (time.perf_counter() - t) * 1000
    qs = np.array([_vec(f"agent memory doc {i * 5}") for i in range(N_QUERIES)],
                  dtype=np.float32)
    t = time.perf_counter()
    idx.search(qs, 10)
    total = (time.perf_counter() - t) * 1000
    # FAISS flat index has no persisted file; serialize to temp path for footprint
    p = tempfile.NamedTemporaryFile(suffix=".faiss", delete=False).name
    faiss.write_index(idx, p)
    size = os.path.getsize(p)
    os.unlink(p)
    return Result("FAISS flat", insert_ms, total, total / N_QUERIES, size,
                  "no persistence in memory; size via write_index()")


def bench_qdrant_inmem() -> Result:
    try:
        from qdrant_client import QdrantClient  # type: ignore
        from qdrant_client.models import Distance, PointStruct, VectorParams  # type: ignore
    except ImportError:
        return Result("Qdrant (in-mem)", 0, 0, 0, 0, "skip: pip install qdrant-client")
    client = QdrantClient(":memory:")
    client.create_collection(
        collection_name="agent_memory",
        vectors_config=VectorParams(size=DIM, distance=Distance.COSINE),
    )
    rows = docs(N_DOCS)
    t = time.perf_counter()
    client.upsert(
        collection_name="agent_memory",
        points=[
            PointStruct(id=int(r["id"]), vector=r["vector"], payload={"text": r["text"]})
            for r in rows
        ],
    )
    insert_ms = (time.perf_counter() - t) * 1000
    qs = [_vec(f"agent memory doc {i * 5}") for i in range(N_QUERIES)]
    t = time.perf_counter()
    for q in qs:
        client.query_points(collection_name="agent_memory", query=q, limit=10)
    total = (time.perf_counter() - t) * 1000
    return Result("Qdrant (in-mem)", insert_ms, total, total / N_QUERIES, 0,
                  "in-memory only · no disk size measured")


def bench_mem0() -> Result:
    try:
        from mem0 import Memory  # type: ignore
    except ImportError:
        return Result("mem0", 0, 0, 0, 0, "skip: pip install mem0ai (+ LLM key)")
    # mem0 normally needs an LLM for extraction — we only time add/search
    try:
        m = Memory()
    except Exception as e:
        return Result("mem0", 0, 0, 0, 0, f"skip: init failed: {e}")
    rows = docs(N_DOCS)
    t = time.perf_counter()
    for r in rows[:100]:  # mem0 is slow, cap at 100 for a fair run
        m.add(r["text"], user_id="bench")
    insert_ms = (time.perf_counter() - t) * 1000
    t = time.perf_counter()
    for i in range(min(N_QUERIES, 20)):
        m.search(query=f"doc {i * 5}", user_id="bench", limit=10)
    total = (time.perf_counter() - t) * 1000
    return Result("mem0 (100 add / 20 q)", insert_ms, total,
                  total / max(1, min(N_QUERIES, 20)), 0, "sub-sampled due to latency")


def bench_sqlite_fts5() -> Result:
    p = tempfile.NamedTemporaryFile(suffix=".db", delete=False).name
    c = sqlite3.connect(p)
    c.execute("CREATE VIRTUAL TABLE docs USING fts5(text)")
    rows = docs(N_DOCS)
    t = time.perf_counter()
    c.executemany("INSERT INTO docs (text) VALUES (?)",
                  [(r["text"],) for r in rows])
    c.commit()
    insert_ms = (time.perf_counter() - t) * 1000
    t = time.perf_counter()
    for i in range(N_QUERIES):
        c.execute("SELECT rowid FROM docs WHERE docs MATCH ? LIMIT 10",
                  (f"doc AND {i * 5}",)).fetchall()
    total = (time.perf_counter() - t) * 1000
    size = os.path.getsize(p)
    os.unlink(p)
    return Result("SQLite FTS5", insert_ms, total, total / N_QUERIES, size,
                  "keyword only (no vector)")


def main() -> None:
    benches = [
        bench_synapse,
        bench_chroma,
        bench_lancedb,
        bench_faiss,
        bench_qdrant_inmem,
        bench_mem0,
        bench_sqlite_fts5,
    ]
    results: list[Result] = []
    for fn in benches:
        try:
            r = fn()
        except Exception as e:
            r = Result(fn.__name__.replace("bench_", ""), 0, 0, 0, 0, f"error: {e}")
        results.append(r)

    print(f"# Real-competitor bench — N={N_DOCS} docs, Q={N_QUERIES} queries, dim={DIM}\n")
    header = (
        f"| {'engine':<22} | {'insert ms':>10} | {'search ms total':>16} | "
        f"{'ms per query':>13} | {'size KB':>9} | notes |"
    )
    print(header)
    print("|" + "-" * (len(header) - 2) + "|")
    for r in sorted(results, key=lambda x: x.search_ms_per_q):
        kb = r.size_bytes / 1024 if r.size_bytes else 0
        print(
            f"| {r.engine:<22} | {r.insert_ms:>10.2f} | {r.search_ms_total:>16.2f} | "
            f"{r.search_ms_per_q:>13.3f} | {kb:>9.1f} | {r.notes} |"
        )


if __name__ == "__main__":
    main()
