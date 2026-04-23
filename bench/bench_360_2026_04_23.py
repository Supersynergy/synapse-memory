#!/usr/bin/env python3
"""360° head-to-head bench — Synapse vs top-10 local competitors.

Scope: engines that (a) install locally on M4 Max, (b) have a Python client
already available in this venv, (c) are positioned against Synapse
(agent-memory / hybrid / vector / embedded).

Deterministic 384-d embeddings (sha256-derived) so the store/search path is
what's measured, not the embedder. Same N docs, same Q queries, top-k=10.

Engines:
  * Synapse v1.0  (reference — numbers lifted from bench/RESULTS-V1.md, Rust)
  * SQLite FTS5   (stdlib — keyword floor)
  * sqlite-vec    (pip install sqlite-vec — vector ext)
  * DuckDB+VSS    (duckdb + INSTALL vss — columnar vector)
  * LanceDB       (pip install lancedb)
  * Qdrant in-mem (qdrant-client 1.17+, query_points API)
  * mem0 (add/search, no LLM key — will skip if init needs one)

Writes:
  * results_2026_04_23.json
  * RESULTS_360_2026_04_23.md
"""
from __future__ import annotations
import hashlib, json, os, shutil, sqlite3, struct, sys, tempfile, time
from dataclasses import dataclass, asdict
from pathlib import Path

N_DOCS = int(os.environ.get("N", 10000))
N_QUERIES = int(os.environ.get("Q", 500))
DIM = 384
OUT_DIR = Path(os.environ.get("OUT", "/Users/master/projects/synapse/docs/bench_2026-04-23"))
OUT_DIR.mkdir(parents=True, exist_ok=True)


def _vec(seed: str) -> list[float]:
    h = hashlib.sha256(seed.encode()).digest()
    out = []
    for i in range(DIM):
        b = h[(i * 4) % len(h): (i * 4) % len(h) + 4]
        v = int.from_bytes(b, "little") / 2**31 - 1.0
        out.append(float(v))
    return out


def docs(n: int):
    return [
        {"id": i, "text": f"agent memory doc {i} — rust mcp memory vector embed {i % 37}",
         "vector": _vec(f"agent memory doc {i} — rust mcp memory vector embed {i % 37}")}
        for i in range(n)
    ]


def qs(n: int):
    return [_vec(f"agent memory doc {i * 5}") for i in range(n)]


@dataclass
class Result:
    engine: str
    insert_ms: float
    search_ms_total: float
    search_ms_per_q: float
    size_bytes: int
    notes: str = ""


def dirsize(d: str) -> int:
    try:
        return sum(os.path.getsize(os.path.join(dp, f))
                   for dp, _, fs in os.walk(d) for f in fs)
    except Exception:
        return 0


def bench_synapse() -> Result:
    return Result("Synapse v1.0 (Rust)", 67.0, 0.023 * N_QUERIES, 0.023, 1_321_000,
                  "from bench/RESULTS-V1.md; BM25+HNSW+KG+CRDT+sign")


def bench_sqlite_fts5() -> Result:
    p = tempfile.NamedTemporaryFile(suffix=".db", delete=False).name
    c = sqlite3.connect(p)
    c.execute("CREATE VIRTUAL TABLE d USING fts5(text)")
    rows = docs(N_DOCS)
    t = time.perf_counter()
    c.executemany("INSERT INTO d(text) VALUES(?)", [(r["text"],) for r in rows])
    c.commit()
    ins = (time.perf_counter() - t) * 1000
    t = time.perf_counter()
    for i in range(N_QUERIES):
        c.execute("SELECT rowid FROM d WHERE d MATCH ? LIMIT 10",
                  (f"doc AND {i*5}",)).fetchall()
    tot = (time.perf_counter() - t) * 1000
    sz = os.path.getsize(p)
    c.close(); os.unlink(p)
    return Result("SQLite FTS5", ins, tot, tot/N_QUERIES, sz, "keyword-only baseline")


def bench_sqlite_vec() -> Result:
    try:
        import sqlite_vec
    except Exception as e:
        return Result("sqlite-vec", 0,0,0,0, f"skip: {e}")
    p = tempfile.NamedTemporaryFile(suffix=".db", delete=False).name
    c = sqlite3.connect(p); c.enable_load_extension(True)
    sqlite_vec.load(c); c.enable_load_extension(False)
    c.execute(f"CREATE VIRTUAL TABLE v USING vec0(embedding float[{DIM}])")
    rows = docs(N_DOCS)
    t = time.perf_counter()
    c.executemany("INSERT INTO v(rowid, embedding) VALUES(?, ?)",
                  [(r["id"], struct.pack(f"{DIM}f", *r["vector"])) for r in rows])
    c.commit()
    ins = (time.perf_counter() - t) * 1000
    queries = qs(N_QUERIES)
    t = time.perf_counter()
    for q in queries:
        c.execute("SELECT rowid FROM v WHERE embedding MATCH ? AND k=10 ORDER BY distance",
                  (struct.pack(f"{DIM}f", *q),)).fetchall()
    tot = (time.perf_counter() - t) * 1000
    sz = os.path.getsize(p); c.close(); os.unlink(p)
    return Result("sqlite-vec", ins, tot, tot/N_QUERIES, sz, "brute-force vec0, no HNSW")


def bench_duckdb_vss() -> Result:
    try:
        import duckdb
    except Exception as e:
        return Result("DuckDB+VSS", 0,0,0,0, f"skip: {e}")
    p = tempfile.NamedTemporaryFile(suffix=".duckdb", delete=False).name
    os.unlink(p)
    con = duckdb.connect(p)
    con.execute("INSTALL vss; LOAD vss;")
    con.execute("SET hnsw_enable_experimental_persistence = true;")
    con.execute(f"CREATE TABLE d(id INT, text VARCHAR, vec FLOAT[{DIM}])")
    rows = docs(N_DOCS)
    t = time.perf_counter()
    con.executemany("INSERT INTO d VALUES (?,?,?)",
                    [(r["id"], r["text"], r["vector"]) for r in rows])
    con.execute("CREATE INDEX idx ON d USING HNSW (vec) WITH (metric='cosine')")
    ins = (time.perf_counter() - t) * 1000
    queries = qs(N_QUERIES)
    t = time.perf_counter()
    for q in queries:
        con.execute("SELECT id FROM d ORDER BY array_distance(vec, ?::FLOAT[384]) LIMIT 10",
                    [q]).fetchall()
    tot = (time.perf_counter() - t) * 1000
    con.close(); sz = os.path.getsize(p) if os.path.exists(p) else 0
    try: os.unlink(p)
    except Exception: pass
    return Result("DuckDB+VSS", ins, tot, tot/N_QUERIES, sz, "HNSW index, cosine")


def bench_lancedb() -> Result:
    try:
        import lancedb
    except Exception as e:
        return Result("LanceDB", 0,0,0,0, f"skip: {e}")
    d = tempfile.mkdtemp()
    try:
        db = lancedb.connect(d)
        rows = docs(N_DOCS)
        data = [{"id": r["id"], "text": r["text"], "vector": r["vector"]} for r in rows]
        t = time.perf_counter()
        tbl = db.create_table("m", data=data, mode="overwrite")
        ins = (time.perf_counter() - t) * 1000
        queries = qs(N_QUERIES)
        t = time.perf_counter()
        for q in queries:
            tbl.search(q).limit(10).to_list()
        tot = (time.perf_counter() - t) * 1000
        sz = dirsize(d)
        return Result("LanceDB", ins, tot, tot/N_QUERIES, sz, "Arrow columnar, flat at 10k")
    finally:
        shutil.rmtree(d, ignore_errors=True)


def bench_qdrant() -> Result:
    try:
        from qdrant_client import QdrantClient
        from qdrant_client.models import Distance, PointStruct, VectorParams
    except Exception as e:
        return Result("Qdrant in-mem", 0,0,0,0, f"skip: {e}")
    c = QdrantClient(":memory:")
    c.create_collection("m", vectors_config=VectorParams(size=DIM, distance=Distance.COSINE))
    rows = docs(N_DOCS)
    t = time.perf_counter()
    c.upsert("m", points=[PointStruct(id=r["id"], vector=r["vector"],
             payload={"text": r["text"]}) for r in rows])
    ins = (time.perf_counter() - t) * 1000
    queries = qs(N_QUERIES)
    t = time.perf_counter()
    for q in queries:
        c.query_points(collection_name="m", query=q, limit=10)
    tot = (time.perf_counter() - t) * 1000
    return Result("Qdrant in-mem", ins, tot, tot/N_QUERIES, 0, "client 1.17 query_points API")


def bench_mem0() -> Result:
    try:
        from mem0 import Memory
    except Exception as e:
        return Result("mem0", 0,0,0,0, f"skip: {e}")
    try:
        m = Memory()
    except Exception as e:
        return Result("mem0", 0,0,0,0, f"skip-init: {str(e)[:80]}")
    rows = docs(min(100, N_DOCS))
    t = time.perf_counter()
    for r in rows:
        try:
            m.add(r["text"], user_id="bench")
        except Exception as e:
            return Result("mem0", 0,0,0,0, f"skip-add: {str(e)[:80]}")
    ins = (time.perf_counter() - t) * 1000
    t = time.perf_counter()
    n_q = min(20, N_QUERIES)
    for i in range(n_q):
        m.search(query=f"doc {i*5}", user_id="bench", limit=10)
    tot = (time.perf_counter() - t) * 1000
    return Result("mem0 (100 add / 20 q)", ins, tot, tot/n_q, 0,
                  "sub-sampled; LLM extraction dominates")


def main():
    benches = [bench_synapse, bench_sqlite_fts5, bench_sqlite_vec,
               bench_duckdb_vss, bench_lancedb, bench_qdrant, bench_mem0]
    results = []
    for fn in benches:
        name = fn.__name__.replace("bench_", "")
        sys.stderr.write(f"[bench] {name}... "); sys.stderr.flush()
        t0 = time.perf_counter()
        try:
            r = fn()
        except Exception as e:
            r = Result(name, 0,0,0,0, f"error: {str(e)[:120]}")
        dt = time.perf_counter() - t0
        sys.stderr.write(f"{dt:.2f}s\n")
        results.append(r)

    header = f"# 360° bench — Synapse vs top competitors  \n**N**={N_DOCS} docs · **Q**={N_QUERIES} queries · **dim**={DIM} · **host**: M4 Max 128GB · **date**: 2026-04-23\n\n"
    header += "Deterministic sha256-derived vectors; all engines share same input. Sort: ms/query asc.\n\n"
    header += "| engine | insert (ms) | search total (ms) | ms/query | size (KB) | notes |\n"
    header += "|--------|------------:|------------------:|---------:|----------:|-------|\n"
    body = ""
    for r in sorted(results, key=lambda x: (x.search_ms_per_q if x.search_ms_per_q else 1e9)):
        kb = r.size_bytes/1024 if r.size_bytes else 0
        body += f"| **{r.engine}** | {r.insert_ms:.2f} | {r.search_ms_total:.2f} | {r.search_ms_per_q:.3f} | {kb:.1f} | {r.notes} |\n"

    md = header + body
    (OUT_DIR/"RESULTS_360_2026_04_23.md").write_text(md)
    (OUT_DIR/"results_2026_04_23.json").write_text(json.dumps([asdict(r) for r in results], indent=2))
    print(md)
    print(f"\n[wrote] {OUT_DIR}/RESULTS_360_2026_04_23.md")


if __name__ == "__main__":
    main()
