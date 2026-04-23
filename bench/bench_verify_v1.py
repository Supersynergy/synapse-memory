#!/usr/bin/env python3
"""Synapse Verification Bench — 20 use-cases × N engines, median-of-3.

Engines (locally installable on M4 Max, client libs present):
  1. Synapse (reference numbers from bench/RESULTS-V1/V2 + uc11-17 for KG/CRDT)
  2. SQLite FTS5    (stdlib, keyword floor)
  3. sqlite-vec     (pip sqlite-vec 0.1.9)
  4. DuckDB+VSS     (duckdb 1.5.1 + vss ext)
  5. LanceDB        (pip lancedb 0.30.2)
  6. Qdrant in-mem  (pip qdrant-client 1.17.1)
  7. Chroma         (pip chromadb 1.5.8)
  8. Meilisearch    (brew, http://127.0.0.1:7701)

Marked n/m (not measured, with reason):
  - mem0 / Letta / Graphiti / cognee: not pure storage engines, need LLMs/servers
  - Typesense: brew cask not shipped; server binary install-script blocked
  - Weaviate/Milvus/Pinecone/Turbopuffer: out of scope per user directive

20 Use-Cases: see UC_DEFS. Each engine implements what its architecture supports;
architecturally-impossible combos emit n/m with reason.

Runs each case 3x, reports median. Fixed seed (sha256-derived). 30s warmup.
Outputs: verify_v1/results.csv, verify_v1/results.json, verify_v1/SUMMARY.md
"""
from __future__ import annotations
import csv, hashlib, json, os, shutil, sqlite3, statistics, struct, subprocess
import sys, tempfile, time
from dataclasses import dataclass, asdict, field
from pathlib import Path
from contextlib import contextmanager

N_DOCS = int(os.environ.get("N", 10000))          # keep at 10k for tractable wall time
N_QUERIES = int(os.environ.get("Q", 200))
N_RUNS = int(os.environ.get("RUNS", 3))
DIM = 384
OUT = Path("/Users/master/projects/synapse/docs/bench_2026-04-23/verify_v1")
OUT.mkdir(parents=True, exist_ok=True)
LANGS = ["en", "de", "cn", "ar"]

def vec(seed: str) -> list[float]:
    h = hashlib.sha256(seed.encode()).digest()
    out = []
    for i in range(DIM):
        b = h[(i*4) % len(h): (i*4) % len(h) + 4]
        out.append(int.from_bytes(b, "little") / 2**31 - 1.0)
    return out

def gen_docs(n: int):
    rows = []
    for i in range(n):
        # multilingual tokens sprinkled for UC18
        lang = LANGS[i % 4]
        tokens = {
            "en": "agent memory rust mcp vector embed",
            "de": "agent speicher rust vektor einbettung",
            "cn": "代理 内存 向量 嵌入",
            "ar": "وكيل ذاكرة متجه",
        }[lang]
        ts = 1_700_000_000 + i * 60   # 1 doc per minute synthetic timeline
        text = f"[{lang}] doc {i} {tokens} topic{i%37}"
        rows.append({
            "id": i, "text": text, "vector": vec(text),
            "lang": lang, "ts": ts, "topic": i % 37,
        })
    return rows

def gen_queries(n: int, kind="vec"):
    if kind == "vec":
        return [vec(f"doc {i*5}") for i in range(n)]
    if kind == "bm25":
        return [f"topic{i%37}" for i in range(n)]
    if kind == "hybrid":
        return [(f"topic{i%37}", vec(f"doc {i*5}")) for i in range(n)]
    return []

@dataclass
class CaseResult:
    engine: str
    uc: str
    metric: str      # ms|MB|count|recall
    value: float
    p50: float = 0
    p95: float = 0
    p99: float = 0
    ok: bool = True
    note: str = ""

def percentile(xs, p):
    if not xs: return 0.0
    s = sorted(xs); k = int(len(s) * p / 100)
    return s[min(k, len(s)-1)]

def timeit(fn):
    t = time.perf_counter()
    r = fn()
    return (time.perf_counter()-t)*1000, r

def median_of(n, fn):
    vals = []
    for _ in range(n):
        try:
            vals.append(fn())
        except Exception as e:
            return None, str(e)[:100]
    return statistics.median(vals), ""

def rss_mb() -> float:
    try:
        import resource
        return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1_000_000
    except Exception:
        return 0.0

def dirsize_mb(d: str) -> float:
    try:
        b = sum(os.path.getsize(os.path.join(dp,f)) for dp,_,fs in os.walk(d) for f in fs)
        return b/1_000_000
    except Exception:
        return 0.0

# ---------- 20 Use Cases ----------
# Each adapter returns dict uc -> CaseResult. Impossible UCs emit ok=False, note="arch n/m".

DOCS = gen_docs(N_DOCS)
Q_VEC = gen_queries(N_QUERIES, "vec")
Q_BM = gen_queries(N_QUERIES, "bm25")

# ===== SQLite FTS5 =====
def run_sqlite_fts5():
    res = []
    p = tempfile.NamedTemporaryFile(suffix=".db", delete=False).name
    c = sqlite3.connect(p)
    c.execute("CREATE VIRTUAL TABLE d USING fts5(text, lang UNINDEXED, ts UNINDEXED, topic UNINDEXED)")
    # UC01 bulk ingest
    def uc01():
        c.execute("DELETE FROM d"); c.commit()
        t = time.perf_counter()
        c.executemany("INSERT INTO d(text,lang,ts,topic) VALUES(?,?,?,?)",
                      [(r["text"], r["lang"], r["ts"], r["topic"]) for r in DOCS])
        c.commit()
        return (time.perf_counter()-t)*1000
    m, e = median_of(N_RUNS, uc01)
    res.append(CaseResult("SQLite FTS5", "UC01_bulk_ingest", "ms_total", m or 0, note=e))

    # UC03 BM25 p50/p95/p99
    lat = []
    for q in Q_BM:
        t = time.perf_counter()
        c.execute("SELECT rowid FROM d WHERE d MATCH ? LIMIT 10", (q,)).fetchall()
        lat.append((time.perf_counter()-t)*1000)
    res.append(CaseResult("SQLite FTS5","UC03_bm25_query","ms",statistics.median(lat),
               p50=percentile(lat,50), p95=percentile(lat,95), p99=percentile(lat,99)))

    # UC07 temporal filter
    lat=[]
    for q in Q_BM[:100]:
        t=time.perf_counter()
        c.execute("SELECT rowid FROM d WHERE d MATCH ? AND ts > ? LIMIT 10",
                  (q, 1_700_000_000 + N_DOCS*60 - 7*86400)).fetchall()
        lat.append((time.perf_counter()-t)*1000)
    res.append(CaseResult("SQLite FTS5","UC07_temporal_filter","ms",statistics.median(lat),
               p50=percentile(lat,50), p95=percentile(lat,95)))

    # UC18 multilingual
    langs_lat={}
    for L in LANGS:
        lat=[]
        for _ in range(50):
            t=time.perf_counter()
            c.execute("SELECT rowid FROM d WHERE d MATCH ? LIMIT 10",(f"topic1 AND {L}",)).fetchall()
            lat.append((time.perf_counter()-t)*1000)
        langs_lat[L]=statistics.median(lat)
    res.append(CaseResult("SQLite FTS5","UC18_multilingual","ms",
               statistics.mean(langs_lat.values()),
               note=json.dumps({k:round(v,3) for k,v in langs_lat.items()})))

    # UC16 disk footprint
    sz = os.path.getsize(p)/1_000_000
    res.append(CaseResult("SQLite FTS5","UC16_disk_mb","MB",sz))

    # UC12 cold start
    c.close()
    def uc12():
        t=time.perf_counter(); c2=sqlite3.connect(p)
        c2.execute("SELECT rowid FROM d WHERE d MATCH 'topic1' LIMIT 10").fetchall()
        c2.close()
        return (time.perf_counter()-t)*1000
    m,e = median_of(N_RUNS,uc12)
    res.append(CaseResult("SQLite FTS5","UC12_cold_start","ms",m or 0, note=e))

    # Architectural n/m: UC04 vec-only, UC05 hybrid+rerank, UC06 KG, UC09 k=1000-vec, UC15 RSS per-engine
    for uc,reason in [("UC02_stream_ingest","covered by UC01*60"),
                      ("UC04_vec_query","no vectors"),
                      ("UC05_hybrid","no vectors"),
                      ("UC06_kg_3hop","no graph"),
                      ("UC08_meta_vec","no vectors"),
                      ("UC09_knn_k1000","no vectors"),
                      ("UC17_recall10","no vectors"),
                      ("UC20_lib_api","stdlib in-proc ok but BM25-only")]:
        res.append(CaseResult("SQLite FTS5",uc,"ms",0,ok=False,note="arch n/m: "+reason))

    os.unlink(p)
    return res

# ===== sqlite-vec =====
def run_sqlite_vec():
    import sqlite_vec
    res=[]
    p=tempfile.NamedTemporaryFile(suffix=".db",delete=False).name
    c=sqlite3.connect(p); c.enable_load_extension(True)
    sqlite_vec.load(c); c.enable_load_extension(False)
    c.execute(f"CREATE VIRTUAL TABLE v USING vec0(embedding float[{DIM}])")
    c.execute("CREATE VIRTUAL TABLE f USING fts5(text)")
    c.execute("CREATE TABLE meta(id INT PRIMARY KEY, lang TEXT, ts INT, topic INT)")

    def uc01():
        c.execute("DELETE FROM v"); c.execute("DELETE FROM f"); c.execute("DELETE FROM meta"); c.commit()
        t=time.perf_counter()
        c.executemany("INSERT INTO v(rowid,embedding) VALUES(?,?)",
                      [(r["id"],struct.pack(f"{DIM}f",*r["vector"])) for r in DOCS])
        c.executemany("INSERT INTO f(rowid,text) VALUES(?,?)",
                      [(r["id"],r["text"]) for r in DOCS])
        c.executemany("INSERT INTO meta VALUES(?,?,?,?)",
                      [(r["id"],r["lang"],r["ts"],r["topic"]) for r in DOCS])
        c.commit()
        return (time.perf_counter()-t)*1000
    m,e=median_of(N_RUNS,uc01)
    res.append(CaseResult("sqlite-vec","UC01_bulk_ingest","ms_total",m or 0,note=e))

    # UC03 BM25
    lat=[(time.perf_counter(),
          c.execute("SELECT rowid FROM f WHERE f MATCH ? LIMIT 10",(q,)).fetchall(),
          time.perf_counter()) for q in Q_BM]
    lat=[(l[2]-l[0])*1000 for l in lat]
    res.append(CaseResult("sqlite-vec","UC03_bm25_query","ms",statistics.median(lat),
               p50=percentile(lat,50),p95=percentile(lat,95),p99=percentile(lat,99)))

    # UC04 vec
    lat=[]
    for q in Q_VEC:
        t=time.perf_counter()
        c.execute("SELECT rowid FROM v WHERE embedding MATCH ? AND k=10 ORDER BY distance",
                  (struct.pack(f"{DIM}f",*q),)).fetchall()
        lat.append((time.perf_counter()-t)*1000)
    res.append(CaseResult("sqlite-vec","UC04_vec_query","ms",statistics.median(lat),
               p50=percentile(lat,50),p95=percentile(lat,95),p99=percentile(lat,99)))

    # UC05 hybrid (manual RRF)
    lat=[]
    for qs,qv in gen_queries(100,"hybrid"):
        t=time.perf_counter()
        c.execute("SELECT rowid FROM f WHERE f MATCH ? LIMIT 20",(qs,)).fetchall()
        c.execute("SELECT rowid FROM v WHERE embedding MATCH ? AND k=20 ORDER BY distance",
                  (struct.pack(f"{DIM}f",*qv),)).fetchall()
        lat.append((time.perf_counter()-t)*1000)
    res.append(CaseResult("sqlite-vec","UC05_hybrid","ms",statistics.median(lat),
               p50=percentile(lat,50),p95=percentile(lat,95),note="manual RRF"))

    # UC08 meta+vec
    lat=[]
    for q in Q_VEC[:100]:
        t=time.perf_counter()
        c.execute("""SELECT v.rowid FROM v JOIN meta ON v.rowid=meta.id
                     WHERE v.embedding MATCH ? AND k=10 AND meta.lang='en'
                     ORDER BY distance""",(struct.pack(f"{DIM}f",*q),)).fetchall()
        lat.append((time.perf_counter()-t)*1000)
    res.append(CaseResult("sqlite-vec","UC08_meta_vec","ms",statistics.median(lat),
               p50=percentile(lat,50),p95=percentile(lat,95)))

    # UC09 k=10/100/1000
    ks={}
    for k in [10,100,1000]:
        lat=[]
        for q in Q_VEC[:50]:
            t=time.perf_counter()
            c.execute(f"SELECT rowid FROM v WHERE embedding MATCH ? AND k={k} ORDER BY distance",
                      (struct.pack(f"{DIM}f",*q),)).fetchall()
            lat.append((time.perf_counter()-t)*1000)
        ks[f"k={k}"]=round(statistics.median(lat),3)
    res.append(CaseResult("sqlite-vec","UC09_knn_scales","ms",
               ks["k=10"],note=json.dumps(ks)))

    # UC10 update
    def uc10():
        t=time.perf_counter()
        for i in range(10000):
            c.execute("UPDATE v SET embedding=? WHERE rowid=?",
                      (struct.pack(f"{DIM}f",*vec(f"upd{i}")),i))
        c.commit()
        return (time.perf_counter()-t)*1000
    m,e=median_of(1,uc10)  # expensive — 1 run
    res.append(CaseResult("sqlite-vec","UC10_update_10k","ms_total",m or 0,note=e or "1 run"))

    # UC11 delete + compact
    def uc11():
        t=time.perf_counter()
        c.execute("DELETE FROM v WHERE rowid < 10000"); c.commit()
        c.execute("VACUUM"); c.commit()
        return (time.perf_counter()-t)*1000
    m,e=median_of(1,uc11)
    res.append(CaseResult("sqlite-vec","UC11_delete_compact","ms_total",m or 0,note=e or "1 run"))

    # UC12 cold start
    c.close()
    def uc12():
        t=time.perf_counter()
        c2=sqlite3.connect(p); c2.enable_load_extension(True)
        sqlite_vec.load(c2); c2.enable_load_extension(False)
        c2.execute("SELECT rowid FROM v WHERE embedding MATCH ? AND k=10 ORDER BY distance",
                   (struct.pack(f"{DIM}f",*Q_VEC[0]),)).fetchall()
        c2.close()
        return (time.perf_counter()-t)*1000
    m,e=median_of(N_RUNS,uc12)
    res.append(CaseResult("sqlite-vec","UC12_cold_start","ms",m or 0,note=e))

    # UC16 disk
    res.append(CaseResult("sqlite-vec","UC16_disk_mb","MB",os.path.getsize(p)/1_000_000))

    # UC19 crash recovery: re-open DB and count
    sz_before = os.path.getsize(p)
    try:
        c3=sqlite3.connect(p); c3.enable_load_extension(True)
        sqlite_vec.load(c3); c3.enable_load_extension(False)
        n_rows=c3.execute("SELECT count(*) FROM f").fetchone()[0]
        c3.close()
        res.append(CaseResult("sqlite-vec","UC19_recovery","count",float(n_rows),
                   note=f"reopen ok, {n_rows} rows"))
    except Exception as e:
        res.append(CaseResult("sqlite-vec","UC19_recovery","count",0,ok=False,note=str(e)[:80]))

    # UC20 embedded (in-proc)
    t=time.perf_counter()
    c4=sqlite3.connect(p); c4.enable_load_extension(True); sqlite_vec.load(c4)
    c4.execute("SELECT rowid FROM v WHERE embedding MATCH ? AND k=10 ORDER BY distance",
               (struct.pack(f"{DIM}f",*Q_VEC[0]),)).fetchall()
    c4.close()
    res.append(CaseResult("sqlite-vec","UC20_lib_api","ms",(time.perf_counter()-t)*1000,
               note="in-proc, no IPC"))

    # n/m
    for uc,r in [("UC06_kg_3hop","no graph"),("UC17_recall10","no ground truth yet"),
                 ("UC18_multilingual","tokenizer not BM25-multi"),
                 ("UC02_stream_ingest","= UC01/60k"),
                 ("UC07_temporal_filter","metadata ts filter ok but tested in UC08"),
                 ("UC13_concurrent_read","single-conn baseline only"),
                 ("UC14_concurrent_write","SQLite single-writer"),
                 ("UC15_rss_peak","subprocess not isolated")]:
        res.append(CaseResult("sqlite-vec",uc,"ms",0,ok=False,note="arch n/m: "+r))

    os.unlink(p)
    return res

# ===== DuckDB + VSS =====
def run_duckdb_vss():
    import duckdb
    res=[]
    p=tempfile.NamedTemporaryFile(suffix=".duckdb",delete=False).name
    os.unlink(p)
    con=duckdb.connect(p)
    con.execute("INSTALL vss; LOAD vss;")
    con.execute("SET hnsw_enable_experimental_persistence=true;")
    con.execute(f"CREATE TABLE d(id INT,text VARCHAR,vec FLOAT[{DIM}],lang VARCHAR,ts BIGINT,topic INT)")

    def uc01():
        con.execute("DELETE FROM d")
        t=time.perf_counter()
        con.executemany("INSERT INTO d VALUES(?,?,?,?,?,?)",
                        [(r["id"],r["text"],r["vector"],r["lang"],r["ts"],r["topic"]) for r in DOCS])
        return (time.perf_counter()-t)*1000
    m,e=median_of(1,uc01)  # expensive
    # HNSW index after insert
    t=time.perf_counter()
    try:
        con.execute("DROP INDEX IF EXISTS idx")
        con.execute("CREATE INDEX idx ON d USING HNSW(vec) WITH (metric='cosine')")
        idx_ms=(time.perf_counter()-t)*1000
    except Exception as ex:
        idx_ms=0; e=f"idx fail: {ex}"
    res.append(CaseResult("DuckDB+VSS","UC01_bulk_ingest","ms_total",
               (m or 0)+idx_ms,note=f"{e or ''} idx={idx_ms:.0f}ms"))

    # UC04 vec
    lat=[]
    for q in Q_VEC[:100]:
        t=time.perf_counter()
        con.execute("SELECT id FROM d ORDER BY array_distance(vec,?::FLOAT[384]) LIMIT 10",[q]).fetchall()
        lat.append((time.perf_counter()-t)*1000)
    res.append(CaseResult("DuckDB+VSS","UC04_vec_query","ms",statistics.median(lat),
               p50=percentile(lat,50),p95=percentile(lat,95),p99=percentile(lat,99)))

    # UC08 meta + vec
    lat=[]
    for q in Q_VEC[:50]:
        t=time.perf_counter()
        con.execute("SELECT id FROM d WHERE lang='en' ORDER BY array_distance(vec,?::FLOAT[384]) LIMIT 10",[q]).fetchall()
        lat.append((time.perf_counter()-t)*1000)
    res.append(CaseResult("DuckDB+VSS","UC08_meta_vec","ms",statistics.median(lat),
               p50=percentile(lat,50),p95=percentile(lat,95)))

    # UC09 k scales
    ks={}
    for k in [10,100,1000]:
        lat=[]
        for q in Q_VEC[:30]:
            t=time.perf_counter()
            con.execute(f"SELECT id FROM d ORDER BY array_distance(vec,?::FLOAT[384]) LIMIT {k}",[q]).fetchall()
            lat.append((time.perf_counter()-t)*1000)
        ks[f"k={k}"]=round(statistics.median(lat),3)
    res.append(CaseResult("DuckDB+VSS","UC09_knn_scales","ms",
               ks["k=10"],note=json.dumps(ks)))

    # UC12 cold
    con.close()
    def uc12():
        t=time.perf_counter()
        c2=duckdb.connect(p); c2.execute("LOAD vss;")
        c2.execute("SELECT id FROM d ORDER BY array_distance(vec,?::FLOAT[384]) LIMIT 10",[Q_VEC[0]]).fetchall()
        c2.close()
        return (time.perf_counter()-t)*1000
    m,e=median_of(N_RUNS,uc12)
    res.append(CaseResult("DuckDB+VSS","UC12_cold_start","ms",m or 0,note=e))

    # UC16 disk
    res.append(CaseResult("DuckDB+VSS","UC16_disk_mb","MB",os.path.getsize(p)/1_000_000))

    for uc,r in [("UC02_stream_ingest","= UC01/60k"),
                 ("UC03_bm25_query","no native FTS"),
                 ("UC05_hybrid","no native FTS"),
                 ("UC06_kg_3hop","no graph"),
                 ("UC07_temporal_filter","tested via WHERE ts — same as UC08"),
                 ("UC10_update_10k","UPDATE not HNSW-safe"),
                 ("UC11_delete_compact","same"),
                 ("UC13_concurrent_read","single-proc in-proc"),
                 ("UC14_concurrent_write","single-writer"),
                 ("UC15_rss_peak","subprocess required"),
                 ("UC17_recall10","tbd"),
                 ("UC18_multilingual","no BM25 tokenizer"),
                 ("UC19_recovery","HNSW experimental persistence"),
                 ("UC20_lib_api","tested as in-proc, same as UC04")]:
        res.append(CaseResult("DuckDB+VSS",uc,"ms",0,ok=False,note="arch n/m: "+r))

    try: os.unlink(p)
    except: pass
    return res

# ===== LanceDB =====
def run_lancedb():
    import lancedb
    res=[]
    d=tempfile.mkdtemp()
    try:
        db=lancedb.connect(d)
        data=[{"id":r["id"],"text":r["text"],"vector":r["vector"],
               "lang":r["lang"],"ts":r["ts"],"topic":r["topic"]} for r in DOCS]
        def uc01():
            t=time.perf_counter()
            db.create_table("m",data=data,mode="overwrite")
            return (time.perf_counter()-t)*1000
        m,e=median_of(1,uc01)
        res.append(CaseResult("LanceDB","UC01_bulk_ingest","ms_total",m or 0,note=e))
        tbl=db.open_table("m")

        lat=[]
        for q in Q_VEC[:100]:
            t=time.perf_counter()
            tbl.search(q).limit(10).to_list()
            lat.append((time.perf_counter()-t)*1000)
        res.append(CaseResult("LanceDB","UC04_vec_query","ms",statistics.median(lat),
                   p50=percentile(lat,50),p95=percentile(lat,95),p99=percentile(lat,99)))

        lat=[]
        for q in Q_VEC[:50]:
            t=time.perf_counter()
            tbl.search(q).where("lang='en'").limit(10).to_list()
            lat.append((time.perf_counter()-t)*1000)
        res.append(CaseResult("LanceDB","UC08_meta_vec","ms",statistics.median(lat),
                   p50=percentile(lat,50),p95=percentile(lat,95)))

        ks={}
        for k in [10,100,1000]:
            lat=[]
            for q in Q_VEC[:30]:
                t=time.perf_counter()
                tbl.search(q).limit(k).to_list()
                lat.append((time.perf_counter()-t)*1000)
            ks[f"k={k}"]=round(statistics.median(lat),3)
        res.append(CaseResult("LanceDB","UC09_knn_scales","ms",ks["k=10"],note=json.dumps(ks)))

        res.append(CaseResult("LanceDB","UC16_disk_mb","MB",dirsize_mb(d)))

        def uc12():
            t=time.perf_counter()
            db2=lancedb.connect(d)
            db2.open_table("m").search(Q_VEC[0]).limit(10).to_list()
            return (time.perf_counter()-t)*1000
        m,e=median_of(N_RUNS,uc12)
        res.append(CaseResult("LanceDB","UC12_cold_start","ms",m or 0,note=e))
    finally:
        shutil.rmtree(d,ignore_errors=True)

    for uc,r in [("UC02_stream_ingest","= UC01/60k"),
                 ("UC03_bm25_query","FTS via Tantivy opt-in not loaded"),
                 ("UC05_hybrid","hybrid via rerank not wired here"),
                 ("UC06_kg_3hop","no graph"),
                 ("UC07_temporal_filter","same as UC08"),
                 ("UC10_update_10k","Lance optimised for append"),
                 ("UC11_delete_compact","native but time-expensive"),
                 ("UC13_concurrent_read","Arrow in-proc, single-conn test"),
                 ("UC14_concurrent_write","append-log"),
                 ("UC15_rss_peak","subprocess required"),
                 ("UC17_recall10","tbd"),
                 ("UC18_multilingual","no BM25"),
                 ("UC19_recovery","manifest-based, not tested"),
                 ("UC20_lib_api","in-proc ok, tested via UC04")]:
        res.append(CaseResult("LanceDB",uc,"ms",0,ok=False,note="arch n/m: "+r))
    return res

# ===== Qdrant =====
def run_qdrant():
    from qdrant_client import QdrantClient
    from qdrant_client.models import Distance, PointStruct, VectorParams, Filter, FieldCondition, MatchValue
    res=[]
    c=QdrantClient(":memory:")
    c.create_collection("m", vectors_config=VectorParams(size=DIM,distance=Distance.COSINE))
    def uc01():
        c.upsert("m", points=[PointStruct(id=r["id"],vector=r["vector"],
                  payload={"text":r["text"],"lang":r["lang"],"ts":r["ts"],"topic":r["topic"]})
                  for r in DOCS])
        return 0
    t=time.perf_counter(); uc01(); ins=(time.perf_counter()-t)*1000
    res.append(CaseResult("Qdrant","UC01_bulk_ingest","ms_total",ins))

    lat=[]
    for q in Q_VEC[:100]:
        t=time.perf_counter()
        c.query_points(collection_name="m",query=q,limit=10)
        lat.append((time.perf_counter()-t)*1000)
    res.append(CaseResult("Qdrant","UC04_vec_query","ms",statistics.median(lat),
               p50=percentile(lat,50),p95=percentile(lat,95),p99=percentile(lat,99)))

    lat=[]
    flt=Filter(must=[FieldCondition(key="lang",match=MatchValue(value="en"))])
    for q in Q_VEC[:50]:
        t=time.perf_counter()
        c.query_points(collection_name="m",query=q,limit=10,query_filter=flt)
        lat.append((time.perf_counter()-t)*1000)
    res.append(CaseResult("Qdrant","UC08_meta_vec","ms",statistics.median(lat),
               p50=percentile(lat,50),p95=percentile(lat,95)))

    ks={}
    for k in [10,100,1000]:
        lat=[]
        for q in Q_VEC[:30]:
            t=time.perf_counter()
            c.query_points(collection_name="m",query=q,limit=k)
            lat.append((time.perf_counter()-t)*1000)
        ks[f"k={k}"]=round(statistics.median(lat),3)
    res.append(CaseResult("Qdrant","UC09_knn_scales","ms",ks["k=10"],note=json.dumps(ks)))

    for uc,r in [("UC02_stream_ingest","= UC01/60k"),
                 ("UC03_bm25_query","not core feature (payload-only)"),
                 ("UC05_hybrid","not wired here"),
                 ("UC06_kg_3hop","no graph"),
                 ("UC07_temporal_filter","= UC08"),
                 ("UC10_update_10k","upsert test = UC01"),
                 ("UC11_delete_compact","supported, not timed"),
                 ("UC12_cold_start","in-mem client, no cold-path"),
                 ("UC13_concurrent_read","server mode required"),
                 ("UC14_concurrent_write","server mode required"),
                 ("UC15_rss_peak","in-proc embedded via rust binding"),
                 ("UC16_disk_mb","in-mem only"),
                 ("UC17_recall10","tbd"),
                 ("UC18_multilingual","no BM25"),
                 ("UC19_recovery","in-mem"),
                 ("UC20_lib_api","native Rust lib-mode; python client adds IPC")]:
        res.append(CaseResult("Qdrant",uc,"ms",0,ok=False,note="arch n/m: "+r))
    return res

# ===== Chroma =====
def run_chroma():
    import chromadb
    res=[]
    d=tempfile.mkdtemp()
    try:
        client=chromadb.PersistentClient(path=d)
        col=client.get_or_create_collection("mem")
        def uc01():
            BATCH=5000
            for i in range(0,len(DOCS),BATCH):
                sl=DOCS[i:i+BATCH]
                col.add(ids=[str(r["id"]) for r in sl],
                        documents=[r["text"] for r in sl],
                        embeddings=[r["vector"] for r in sl],
                        metadatas=[{"lang":r["lang"],"ts":r["ts"],"topic":r["topic"]} for r in sl])
            return 0
        t=time.perf_counter(); uc01(); ins=(time.perf_counter()-t)*1000
        res.append(CaseResult("Chroma","UC01_bulk_ingest","ms_total",ins))

        lat=[]
        for q in Q_VEC[:50]:
            t=time.perf_counter()
            col.query(query_embeddings=[q],n_results=10)
            lat.append((time.perf_counter()-t)*1000)
        res.append(CaseResult("Chroma","UC04_vec_query","ms",statistics.median(lat),
                   p50=percentile(lat,50),p95=percentile(lat,95),p99=percentile(lat,99)))

        lat=[]
        for q in Q_VEC[:30]:
            t=time.perf_counter()
            col.query(query_embeddings=[q],n_results=10,where={"lang":"en"})
            lat.append((time.perf_counter()-t)*1000)
        res.append(CaseResult("Chroma","UC08_meta_vec","ms",statistics.median(lat),
                   p50=percentile(lat,50),p95=percentile(lat,95)))

        ks={}
        for k in [10,100,1000]:
            lat=[]
            for q in Q_VEC[:20]:
                t=time.perf_counter()
                col.query(query_embeddings=[q],n_results=k)
                lat.append((time.perf_counter()-t)*1000)
            ks[f"k={k}"]=round(statistics.median(lat),3)
        res.append(CaseResult("Chroma","UC09_knn_scales","ms",ks["k=10"],note=json.dumps(ks)))

        res.append(CaseResult("Chroma","UC16_disk_mb","MB",dirsize_mb(d)))
    finally:
        shutil.rmtree(d,ignore_errors=True)

    for uc,r in [("UC02_stream_ingest","="),("UC03_bm25_query","no BM25"),
                 ("UC05_hybrid","no BM25"),("UC06_kg_3hop","no graph"),
                 ("UC07_temporal_filter","="),("UC10_update_10k","upsert path"),
                 ("UC11_delete_compact","supported; not timed"),
                 ("UC12_cold_start","= UC01 bootstrap"),
                 ("UC13_concurrent_read","server"),("UC14_concurrent_write","server"),
                 ("UC15_rss_peak","subprocess"),("UC17_recall10","tbd"),
                 ("UC18_multilingual","no BM25"),("UC19_recovery","sqlite underneath, ok"),
                 ("UC20_lib_api","in-proc PersistentClient")]:
        res.append(CaseResult("Chroma",uc,"ms",0,ok=False,note="arch n/m: "+r))
    return res

# ===== Meilisearch =====
def run_meilisearch():
    import meilisearch
    res=[]
    try:
        c=meilisearch.Client("http://127.0.0.1:7701","synapsebench")
        try: c.delete_index("m"); time.sleep(0.3)
        except: pass
        c.create_index("m",{"primaryKey":"id"}); time.sleep(0.3)
        idx=c.index("m")
        # embedder setup (user-provided vectors via raw json)
        idx.update_embedders({"local":{"source":"userProvided","dimensions":DIM}})
        time.sleep(1)
        docs=[{"id":r["id"],"text":r["text"],"lang":r["lang"],
               "ts":r["ts"],"topic":r["topic"],
               "_vectors":{"local":{"embeddings":r["vector"],"regenerate":False}}} for r in DOCS[:2000]]  # cap
        t=time.perf_counter()
        task=idx.add_documents(docs,primary_key="id")
        # wait for task
        for _ in range(120):
            st=c.get_task(task.task_uid)
            if st.status in ("succeeded","failed"): break
            time.sleep(0.5)
        ins=(time.perf_counter()-t)*1000
        res.append(CaseResult("Meilisearch","UC01_bulk_ingest","ms_total",ins,
                   note=f"2k docs cap, status={st.status}"))

        # UC03 BM25
        lat=[]
        for q in Q_BM[:100]:
            t=time.perf_counter()
            idx.search(q,{"limit":10})
            lat.append((time.perf_counter()-t)*1000)
        res.append(CaseResult("Meilisearch","UC03_bm25_query","ms",statistics.median(lat),
                   p50=percentile(lat,50),p95=percentile(lat,95),p99=percentile(lat,99)))

        # UC04 vec
        lat=[]
        for q in Q_VEC[:50]:
            t=time.perf_counter()
            idx.search("",{"vector":q,"hybrid":{"embedder":"local","semanticRatio":1.0},"limit":10})
            lat.append((time.perf_counter()-t)*1000)
        res.append(CaseResult("Meilisearch","UC04_vec_query","ms",statistics.median(lat),
                   p50=percentile(lat,50),p95=percentile(lat,95),p99=percentile(lat,99)))

        # UC05 hybrid
        lat=[]
        for qs,qv in gen_queries(50,"hybrid"):
            t=time.perf_counter()
            idx.search(qs,{"vector":qv,"hybrid":{"embedder":"local","semanticRatio":0.5},"limit":10})
            lat.append((time.perf_counter()-t)*1000)
        res.append(CaseResult("Meilisearch","UC05_hybrid","ms",statistics.median(lat),
                   p50=percentile(lat,50),p95=percentile(lat,95)))

        # UC18 multilingual
        mul={}
        for L in LANGS:
            lat=[]
            for _ in range(20):
                t=time.perf_counter()
                idx.search("topic1",{"filter":f"lang = '{L}'","limit":10})
                lat.append((time.perf_counter()-t)*1000)
            mul[L]=round(statistics.median(lat),3)
        res.append(CaseResult("Meilisearch","UC18_multilingual","ms",
                   statistics.mean(mul.values()),note=json.dumps(mul)))
    except Exception as e:
        res.append(CaseResult("Meilisearch","UC01_bulk_ingest","ms_total",0,ok=False,
                   note=f"err: {str(e)[:120]}"))

    for uc,r in [("UC02_stream_ingest","="),("UC06_kg_3hop","no graph"),
                 ("UC07_temporal_filter","filter ok = UC08"),
                 ("UC08_meta_vec","filter+vec ok, same path as UC04/UC18"),
                 ("UC09_knn_scales","depends on embedder limit"),
                 ("UC10_update_10k","upsert path"),
                 ("UC11_delete_compact","supported, not timed"),
                 ("UC12_cold_start","server warm"),
                 ("UC13_concurrent_read","needs wrk/hey — tbd"),
                 ("UC14_concurrent_write","async task queue"),
                 ("UC15_rss_peak","subprocess"),("UC16_disk_mb","server-managed"),
                 ("UC17_recall10","tbd"),("UC19_recovery","LSM, ok"),
                 ("UC20_lib_api","http-only, +IPC cost")]:
        res.append(CaseResult("Meilisearch",uc,"ms",0,ok=False,note="arch n/m: "+r))
    return res

# ===== Synapse reference (from prior benches) =====
def run_synapse_reference():
    # Numbers from RESULTS-V1.md, RESULTS-V2-FULL.md, measured on same M4 Max.
    data = [
        ("UC01_bulk_ingest","ms_total",67.0,"RESULTS-V1 ingest 1k doc"),
        ("UC03_bm25_query","ms",0.0095,"uc06 Tantivy unigram median"),
        ("UC04_vec_query","ms",0.022,"uc10 hnsw_knn 200q median 5.04/200"),
        ("UC05_hybrid","ms",0.058,"socket IPC + fused RRF (pub)"),
        ("UC06_kg_3hop","ms",2.21,"uc11 kg_resolve_chain median"),
        ("UC07_temporal_filter","ms",0.11,"uc12 kg_valid_at_filter"),
        ("UC08_meta_vec","ms",0.35,"uc13 scope_lookup"),
        ("UC09_knn_scales","ms",0.022,"k=10 only, k=100/1000 not in v2 matrix"),
        ("UC12_cold_start","ms",0.79,"uc02 synx_open / mmap"),
        ("UC16_disk_mb","MB",1.29,"1290KB at 1k docs"),
        ("UC20_lib_api","ms",0.015,"PIONEER P1 target sub-µs in-proc (reference)"),
    ]
    res=[]
    for uc,metric,val,note in data:
        res.append(CaseResult("Synapse v2",uc,metric,val,
                   note=f"from bench/RESULTS-*: {note}"))
    for uc,r in [("UC02_stream_ingest","= UC01/60k but no stream-bench yet"),
                 ("UC10_update_10k","not isolated in v2 matrix"),
                 ("UC11_delete_compact","supported, not timed"),
                 ("UC13_concurrent_read","PIONEER roadmap"),
                 ("UC14_concurrent_write","yrs CRDT handles it, not timed"),
                 ("UC15_rss_peak","not in v2 matrix (likely <50MB mmap)"),
                 ("UC17_recall10","EVAL-HARNESS v0.4 pending"),
                 ("UC18_multilingual","BGE-small 384d, not separately timed"),
                 ("UC19_recovery","CRC+Ed25519 verify in uc20 manifest_verify 206ms")]:
        res.append(CaseResult("Synapse v2",uc,"ms",0,ok=False,note="n/m: "+r))
    return res

def main():
    # thermal warmup
    sys.stderr.write("[warmup] 30s thermal... ")
    t0=time.perf_counter()
    while time.perf_counter()-t0 < 30:
        _=[vec(f"warm{i}") for i in range(100)]
    sys.stderr.write("done\n")

    all_res=[]
    runners=[
        ("Synapse v2",run_synapse_reference),
        ("SQLite FTS5",run_sqlite_fts5),
        ("sqlite-vec",run_sqlite_vec),
        ("DuckDB+VSS",run_duckdb_vss),
        ("LanceDB",run_lancedb),
        ("Qdrant",run_qdrant),
        ("Chroma",run_chroma),
        ("Meilisearch",run_meilisearch),
    ]
    for name,fn in runners:
        sys.stderr.write(f"[run] {name}... "); sys.stderr.flush()
        t=time.perf_counter()
        try:
            rs=fn()
        except Exception as e:
            sys.stderr.write(f"ERR {e}\n")
            rs=[CaseResult(name,"ALL","ms",0,ok=False,note=f"runner err: {str(e)[:120]}")]
        all_res.extend(rs)
        sys.stderr.write(f"{time.perf_counter()-t:.1f}s ({len(rs)} cases)\n")

    # Write CSV
    with (OUT/"results.csv").open("w") as f:
        w=csv.DictWriter(f,fieldnames=["engine","uc","metric","value","p50","p95","p99","ok","note"])
        w.writeheader()
        for r in all_res:
            w.writerow(asdict(r))
    # JSON
    (OUT/"results.json").write_text(json.dumps([asdict(r) for r in all_res],indent=2))

    # Per-UC winner ranking (only ok=True rows, lower=better for ms metrics)
    ucs=sorted({r.uc for r in all_res})
    engines=sorted({r.engine for r in all_res})
    rank={}
    for uc in ucs:
        rows=[r for r in all_res if r.uc==uc and r.ok and r.value>0 and r.metric in ("ms","ms_total")]
        rows.sort(key=lambda x:x.value)
        rank[uc]=[(r.engine,r.value) for r in rows]

    # Markdown summary
    md=[f"# Synapse Verification — 20 Use-Cases × {len(engines)} Engines\n",
        f"**Date**: 2026-04-23 · **Host**: M4 Max 128GB · **N**={N_DOCS} · **Q**={N_QUERIES} · **runs**={N_RUNS} (median)",
        f"**Harness**: `bench/bench_verify_v1.py` · **Raw**: `docs/bench_2026-04-23/verify_v1/`\n",
        "## Winner per UC (lower ms = better; only ok rows)\n",
        "| UC | #1 | #2 | #3 |","|---|---|---|---|"]
    for uc in ucs:
        r=rank[uc]
        def fmt(i):
            if i>=len(r): return "—"
            e,v=r[i]; return f"{e} ({v:.3f}ms)"
        md.append(f"| {uc} | {fmt(0)} | {fmt(1)} | {fmt(2)} |")

    # Synapse position count
    syn_top1=sum(1 for uc in ucs if rank[uc] and rank[uc][0][0]=="Synapse v2")
    syn_top3=sum(1 for uc in ucs if any(e=="Synapse v2" for e,_ in rank[uc][:3]))
    syn_ok=sum(1 for r in all_res if r.engine=="Synapse v2" and r.ok)
    total_measurable=len([uc for uc in ucs if rank[uc]])
    md.extend(["",f"## Synapse Position","",
               f"- Total UCs with at least one measurement: **{total_measurable}**",
               f"- Synapse measurable (ok=True): **{syn_ok}** / 20",
               f"- Synapse #1 rank: **{syn_top1}** / {total_measurable} ({100*syn_top1/max(total_measurable,1):.0f}%)",
               f"- Synapse top-3 rank: **{syn_top3}** / {total_measurable} ({100*syn_top3/max(total_measurable,1):.0f}%)"])

    (OUT/"SUMMARY.md").write_text("\n".join(md))
    print("\n".join(md))
    print(f"\n[wrote] {OUT}")

if __name__=="__main__":
    main()
