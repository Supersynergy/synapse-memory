#!/usr/bin/env python3
"""
synapse-turbo v2 — Ultimate Synapse query engine, M4 Max optimized.

Architecture: 3-tier query resolution
  T1: Pre-computed results dict  → 0.0003ms (hash lookup)
  T2: NumPy SIMD brute-force     → 0.05ms   (NEON-accel matmul, beats sqlite-vec)
  T3: fastembed ONNX (8 threads) → 3.2ms    (only on embedding cache miss)

Daemon mode: uvloop + orjson + in-memory matrix + pre-computed cache
  Cached hybrid query:  ~0.1ms server-side
  New query (embed):    ~3.5ms server-side
  vs Synapse CLI:       ~300ms (1000x slower)

Usage:
    synapse-turbo find   "query" [--limit N]
    synapse-turbo vec    "query" [--limit N]
    synapse-turbo hybrid "query" [--limit N]
    synapse-turbo warm                          # pre-warm all caches
    synapse-turbo stats                         # cache statistics
    synapse-turbo daemon [--port 9477]          # ultimate daemon
    synapse-turbo q      "query" [--limit N]    # query via daemon
    synapse-turbo bench                         # self-benchmark
"""
import sqlite3
import struct
import hashlib
import sys
import os
import time

BRAIN_DB = os.path.expanduser("~/.synapse/brain.db")
CACHE_DB = os.path.expanduser("~/.synapse/emb_cache.db")
EMBED_MODEL = "BAAI/bge-small-en-v1.5"
EMBED_DIM = 384
DAEMON_PORT = 9477
DAEMON_PID = os.path.expanduser("~/.synapse/turbo.pid")
ONNX_THREADS = 8  # Optimal for M4 Max (benchmark-verified)

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# JSON: use orjson if available (5-10x faster), fall back to json
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
try:
    import orjson
    def json_dumps(obj): return orjson.dumps(obj)
    def json_loads(s): return orjson.loads(s)
except ImportError:
    import json
    def json_dumps(obj): return json.dumps(obj).encode()
    def json_loads(s): return json.loads(s)

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Embedding Cache (SQLite, persistent)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def get_cache_db():
    db = sqlite3.connect(CACHE_DB)
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("PRAGMA synchronous=NORMAL")
    db.execute("PRAGMA mmap_size=67108864")
    db.execute("""CREATE TABLE IF NOT EXISTS emb_cache (
        query_hash TEXT PRIMARY KEY,
        query_text TEXT NOT NULL,
        embedding  BLOB NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch()))""")
    return db

def qhash(text):
    return hashlib.blake2b(text.encode(), digest_size=16).hexdigest()

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# fastembed (lazy, 8 threads, session reuse)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

_fe_model = None

def get_fastembed():
    global _fe_model
    if _fe_model is None:
        os.environ["OMP_NUM_THREADS"] = str(ONNX_THREADS)
        from fastembed import TextEmbedding
        _fe_model = TextEmbedding(model_name=EMBED_MODEL, threads=ONNX_THREADS)
    return _fe_model

def embed_text(model, text):
    emb = list(model.embed([text]))[0]
    return struct.pack(f'{EMBED_DIM}f', *emb.tolist())

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Brain DB (sqlite-vec, FTS5)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def get_brain_db():
    import sqlite_vec
    db = sqlite3.connect(BRAIN_DB)
    db.enable_load_extension(True)
    sqlite_vec.load(db)
    db.execute("PRAGMA mmap_size=268435456")
    db.execute("PRAGMA cache_size=-65536")
    db.execute("PRAGMA temp_store=MEMORY")
    return db

def search_fts(db, query, limit=10):
    try:
        return db.execute(
            "SELECT d.id, rank, d.uri, d.title, substr(d.text,1,200) "
            "FROM docs_fts f JOIN docs d ON d.id=f.rowid "
            "WHERE docs_fts MATCH ? ORDER BY rank LIMIT ?",
            (query, limit)).fetchall()
    except Exception:
        return []

def search_vec_sqlite(db, emb_bytes, limit=10):
    return db.execute(
        "SELECT v.id, v.distance, d.uri, d.title, substr(d.text,1,200) "
        "FROM docs_vec v JOIN docs d ON d.id=v.id "
        "WHERE v.embedding MATCH ?1 AND k=?2 ORDER BY v.distance",
        (emb_bytes, limit)).fetchall()

def hybrid_rrf(fts_results, vec_results, limit=10, k=60):
    scores, meta = {}, {}
    for i, r in enumerate(fts_results):
        scores[r[0]] = scores.get(r[0], 0) + 1.0/(k+i+1)
        meta[r[0]] = (r[2], r[3], r[4])
    for i, r in enumerate(vec_results):
        scores[r[0]] = scores.get(r[0], 0) + 1.0/(k+i+1)
        if r[0] not in meta:
            meta[r[0]] = (r[2], r[3], r[4])
    ranked = sorted(scores.items(), key=lambda x: -x[1])[:limit]
    return [(did, sc, *meta[did]) for did, sc in ranked]


# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T2: NumPy in-memory engine (0.05ms, beats sqlite-vec at <50k docs)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

class NumpyVecEngine:
    """In-memory NEON-accelerated brute-force kNN. 4.7x faster than sqlite-vec."""

    def __init__(self, brain_db):
        import numpy as np
        import sqlite_vec
        self.np = np

        db = sqlite3.connect(brain_db)
        db.enable_load_extension(True)
        sqlite_vec.load(db)

        rows = db.execute("SELECT v.id, v.embedding FROM docs_vec v").fetchall()
        self.ids = [r[0] for r in rows]
        raw = np.array([np.frombuffer(r[1], dtype=np.float32) for r in rows])
        norms = np.linalg.norm(raw, axis=1, keepdims=True)
        norms[norms == 0] = 1.0
        self.matrix = raw / norms  # pre-normalized for cosine
        self.doc_count = len(self.ids)

        # Pre-load doc metadata
        meta_rows = db.execute("SELECT id, uri, title, substr(text,1,200) FROM docs").fetchall()
        self.meta = {r[0]: (r[1], r[2], r[3]) for r in meta_rows}
        db.close()

    def search(self, emb_bytes, limit=10):
        np = self.np
        q = np.frombuffer(emb_bytes, dtype=np.float32)
        qn = np.linalg.norm(q)
        if qn == 0:
            return []
        q_norm = q / qn
        sims = self.matrix @ q_norm
        np.nan_to_num(sims, copy=False, nan=-1.0)
        top_idx = np.argpartition(sims, -limit)[-limit:]
        top_idx = top_idx[np.argsort(sims[top_idx])[::-1]]

        results = []
        for idx in top_idx:
            did = self.ids[idx]
            dist = 1.0 - float(sims[idx])
            uri, title, text = self.meta.get(did, (None, None, None))
            results.append((did, dist, uri, title, text))
        return results


# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# CLI commands
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def fmt(idx, doc_id, score, uri, title, text):
    label = title or uri or f"doc#{doc_id}"
    snip = (text or "").replace('\n', ' ').strip()[:120]
    return f"  {idx+1}. [{score:.4f}] {label}\n     {snip}"

def resolve_embedding(query_text, cache_db, fe_model=None):
    """Resolve embedding: cache -> fastembed. Returns (bytes, cached_bool)."""
    h = qhash(query_text)
    row = cache_db.execute("SELECT embedding FROM emb_cache WHERE query_hash=?", (h,)).fetchone()
    if row:
        return row[0], True
    if fe_model is None:
        fe_model = get_fastembed()
    emb_bytes = embed_text(fe_model, query_text)
    cache_db.execute("INSERT OR REPLACE INTO emb_cache (query_hash,query_text,embedding) VALUES (?,?,?)",
                     (h, query_text, emb_bytes))
    cache_db.commit()
    return emb_bytes, False

def cmd_find(query, limit=10):
    t0 = time.perf_counter()
    db = get_brain_db()
    results = search_fts(db, query, limit)
    ms = (time.perf_counter()-t0)*1000
    db.close()
    print(f"FTS5 [{ms:.1f}ms] {len(results)} results")
    for i, (did, rank, uri, title, text) in enumerate(results):
        print(fmt(i, did, abs(rank), uri, title, text))

def cmd_vec(query, limit=10):
    t0 = time.perf_counter()
    cache = get_cache_db()
    emb, cached = resolve_embedding(query, cache)
    ems = (time.perf_counter()-t0)*1000
    db = get_brain_db()
    t1 = time.perf_counter()
    results = search_vec_sqlite(db, emb, limit)
    sms = (time.perf_counter()-t1)*1000
    tot = (time.perf_counter()-t0)*1000
    db.close(); cache.close()
    tag = "cached" if cached else "computed"
    print(f"VEC [{tot:.1f}ms] embed={ems:.1f}ms({tag}) search={sms:.1f}ms | {len(results)} results")
    for i, (did, dist, uri, title, text) in enumerate(results):
        print(fmt(i, did, dist, uri, title, text))

def cmd_hybrid(query, limit=10):
    t0 = time.perf_counter()
    cache = get_cache_db()
    emb, cached = resolve_embedding(query, cache)
    ems = (time.perf_counter()-t0)*1000
    db = get_brain_db()
    t1 = time.perf_counter()
    fts_r = search_fts(db, query, limit*2)
    vec_r = search_vec_sqlite(db, emb, limit*2)
    results = hybrid_rrf(fts_r, vec_r, limit)
    sms = (time.perf_counter()-t1)*1000
    tot = (time.perf_counter()-t0)*1000
    db.close(); cache.close()
    tag = "cached" if cached else "computed"
    print(f"HYBRID [{tot:.1f}ms] embed={ems:.1f}ms({tag}) search={sms:.1f}ms | {len(results)} results")
    for i, (did, score, uri, title, text) in enumerate(results):
        print(fmt(i, did, score, uri, title, text))

def cmd_warm():
    t0 = time.perf_counter()
    model = get_fastembed()
    list(model.embed(["warmup"]))
    print(f"Model warmed in {(time.perf_counter()-t0)*1000:.0f}ms (threads={ONNX_THREADS})")

def cmd_stats():
    if not os.path.exists(CACHE_DB):
        print("No cache yet."); return
    cache = get_cache_db()
    count = cache.execute("SELECT COUNT(*) FROM emb_cache").fetchone()[0]
    cache.close()
    print(f"Cache: {count} embeddings, {os.path.getsize(CACHE_DB)/1024:.0f}KB")
    print(f"Brain: {os.path.getsize(BRAIN_DB)/1024/1024:.1f}MB")

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# ULTIMATE DAEMON: uvloop + orjson + NumPy + 3-tier cache
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def cmd_daemon(port=None):
    port = port or DAEMON_PORT

    # Try uvloop for faster event loop
    try:
        import uvloop
        uvloop.install()
        loop_name = "uvloop"
    except ImportError:
        loop_name = "asyncio"

    import asyncio
    from urllib.parse import urlparse, parse_qs

    print(f"[init] Loading fastembed model (threads={ONNX_THREADS})...", flush=True)
    t0 = time.perf_counter()
    model = get_fastembed()
    list(model.embed(["warmup"]))
    print(f"[init] Model ready: {(time.perf_counter()-t0)*1000:.0f}ms", flush=True)

    print(f"[init] Loading NumPy vector engine...", flush=True)
    t0 = time.perf_counter()
    np_engine = NumpyVecEngine(BRAIN_DB)
    print(f"[init] NumPy engine: {np_engine.doc_count} docs loaded in {(time.perf_counter()-t0)*1000:.0f}ms", flush=True)

    brain = get_brain_db()
    cache = get_cache_db()

    # T1: Pre-compute results for all cached queries
    print(f"[init] Pre-computing results cache...", flush=True)
    t0 = time.perf_counter()
    results_cache = {}  # hash -> serialized JSON response
    emb_mem = {}        # hash -> embedding bytes (in-memory, faster than SQLite)
    for row in cache.execute("SELECT query_hash, query_text, embedding FROM emb_cache").fetchall():
        h, qt, emb = row
        emb_mem[h] = emb

        # Pre-compute hybrid results
        fts_r = search_fts(brain, qt, 20)
        vec_r = np_engine.search(emb, 20)
        hybrid_r = hybrid_rrf(fts_r, vec_r, 10)

        results_cache[h] = {
            "hybrid": [{"id": r[0], "score": r[1], "title": r[3], "text": r[4]} for r in hybrid_r],
            "vec": [{"id": r[0], "distance": r[1], "title": r[3], "text": r[4]} for r in vec_r[:10]],
        }
    pre_ms = (time.perf_counter()-t0)*1000
    print(f"[init] Pre-computed: {len(results_cache)} queries in {pre_ms:.0f}ms", flush=True)

    with open(DAEMON_PID, 'w') as f:
        f.write(str(os.getpid()))

    async def handle_request(reader, writer):
        t0 = time.perf_counter()
        data = await reader.readuntil(b'\r\n\r\n')
        first_line = data.split(b'\r\n')[0].decode()
        # Parse: GET /hybrid?q=...&limit=5 HTTP/1.1
        parts = first_line.split(' ')
        if len(parts) < 2:
            writer.close()
            return
        parsed = urlparse(parts[1])
        params = parse_qs(parsed.query)
        query = params.get('q', [''])[0]
        limit = int(params.get('limit', ['5'])[0])
        mode = parsed.path.strip('/')

        if not query or mode not in ('find', 'vec', 'hybrid'):
            body = b'{"error":"use /find?q=... or /vec?q=... or /hybrid?q=..."}'
            resp = b'HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: ' + str(len(body)).encode() + b'\r\nConnection: close\r\n\r\n' + body
            writer.write(resp)
            await writer.drain()
            writer.close()
            return

        h = qhash(query)

        if mode == 'find':
            results = search_fts(brain, query, limit)
            out = [{"id": r[0], "score": abs(r[1]), "title": r[3], "text": r[4]} for r in results]
        else:
            # T1: Check pre-computed results cache (0.0003ms)
            pre = results_cache.get(h)
            if pre and mode in pre:
                out = pre[mode][:limit]
            else:
                # T2/T3: Need embedding
                emb_bytes = emb_mem.get(h)
                if not emb_bytes:
                    row = cache.execute("SELECT embedding FROM emb_cache WHERE query_hash=?", (h,)).fetchone()
                    if row:
                        emb_bytes = row[0]
                        emb_mem[h] = emb_bytes
                    else:
                        # T3: Compute embedding
                        emb_bytes = embed_text(model, query)
                        emb_mem[h] = emb_bytes
                        cache.execute("INSERT OR REPLACE INTO emb_cache (query_hash,query_text,embedding) VALUES (?,?,?)",
                                      (h, query, emb_bytes))
                        cache.commit()

                if mode == 'vec':
                    # T2: NumPy search
                    results = np_engine.search(emb_bytes, limit)
                    out = [{"id": r[0], "distance": r[1], "title": r[3], "text": r[4]} for r in results]
                else:
                    fts_r = search_fts(brain, query, limit*2)
                    vec_r = np_engine.search(emb_bytes, limit*2)
                    hybrid_r = hybrid_rrf(fts_r, vec_r, limit)
                    out = [{"id": r[0], "score": r[1], "title": r[3], "text": r[4]} for r in hybrid_r]

                # Cache for next time
                if h not in results_cache:
                    results_cache[h] = {}
                results_cache[h][mode] = out

        elapsed = (time.perf_counter()-t0)*1000
        payload = json_dumps({"mode": mode, "query": query, "elapsed_ms": round(elapsed, 3),
                              "count": len(out), "results": out})
        resp = b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ' + str(len(payload)).encode() + b'\r\nConnection: close\r\n\r\n' + payload
        writer.write(resp)
        await writer.drain()
        writer.close()

    async def serve():
        server = await asyncio.start_server(handle_request, '127.0.0.1', port)
        print(f"synapse-turbo v2 daemon on http://127.0.0.1:{port} ({loop_name})", flush=True)
        print(f"  /find?q=...   /vec?q=...   /hybrid?q=...", flush=True)
        print(f"  T1: {len(results_cache)} pre-computed | T2: NumPy {np_engine.doc_count} docs | T3: ONNX {ONNX_THREADS}t", flush=True)
        print(f"  PID: {os.getpid()}", flush=True)
        async with server:
            await server.serve_forever()

    try:
        asyncio.run(serve())
    except KeyboardInterrupt:
        print("\nShutting down.")
    finally:
        brain.close()
        cache.close()
        try:
            os.unlink(DAEMON_PID)
        except OSError:
            pass

def cmd_query_daemon(query, mode="hybrid", limit=5):
    import urllib.request, urllib.parse
    url = f"http://127.0.0.1:{DAEMON_PORT}/{mode}?q={urllib.parse.quote(query)}&limit={limit}"
    try:
        t0 = time.perf_counter()
        raw = urllib.request.urlopen(url, timeout=2).read()
        resp = json_loads(raw)
        total = (time.perf_counter()-t0)*1000
        print(f"{resp['mode'].upper()} [{resp['elapsed_ms']}ms server, {total:.1f}ms e2e] {resp['count']} results")
        for i, r in enumerate(resp['results']):
            label = r.get('title') or f"doc#{r['id']}"
            score = r.get('score') or r.get('distance', 0)
            text = (r.get('text') or '').replace('\n', ' ')[:120]
            print(f"  {i+1}. [{score:.4f}] {label}\n     {text}")
    except Exception as e:
        print(f"Daemon not running. Start: synapse-turbo daemon\nError: {e}")

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Self-benchmark
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def cmd_bench():
    import numpy as np
    print("synapse-turbo v2 — self-benchmark (M4 Max)")
    print("=" * 60)

    brain = get_brain_db()
    cache = get_cache_db()
    np_eng = NumpyVecEngine(BRAIN_DB)

    queries = ["rust web framework", "knowledge base", "embedding cache",
               "agent orchestration", "Claude 4"]

    # Ensure all cached
    model = get_fastembed()
    for q in queries:
        h = qhash(q)
        if not cache.execute("SELECT 1 FROM emb_cache WHERE query_hash=?", (h,)).fetchone():
            emb = embed_text(model, q)
            cache.execute("INSERT INTO emb_cache (query_hash,query_text,embedding) VALUES (?,?,?)", (h,q,emb))
    cache.commit()

    # Pre-computed dict
    pre = {}
    emb_dict = {}
    for q in queries:
        h = qhash(q)
        emb = cache.execute("SELECT embedding FROM emb_cache WHERE query_hash=?", (h,)).fetchone()[0]
        emb_dict[h] = emb
        vec_r = np_eng.search(emb, 10)
        fts_r = search_fts(brain, q, 10)
        pre[h] = hybrid_rrf(fts_r, vec_r, 5)

    def bench(name, fn, n=100):
        # warmup
        for _ in range(5): fn()
        times = []
        for _ in range(n):
            t0 = time.perf_counter()
            fn()
            times.append((time.perf_counter()-t0)*1000)
        times.sort()
        p50 = times[n//2]
        p99 = times[int(n*0.99)]
        print(f"  {name:<40s} min={min(times):.4f}ms p50={p50:.4f}ms p99={p99:.4f}ms")

    q = queries[0]
    h = qhash(q)
    emb = emb_dict[h]

    print("\n--- T1: Pre-computed dict lookup ---")
    bench("dict[hash]", lambda: pre.get(h))

    print("\n--- T2: NumPy brute-force kNN ---")
    bench("numpy cosine top-5", lambda: np_eng.search(emb, 5))

    print("\n--- sqlite-vec kNN ---")
    bench("sqlite-vec top-5", lambda: brain.execute(
        "SELECT v.id,v.distance FROM docs_vec v WHERE v.embedding MATCH ?1 AND k=5", (emb,)).fetchall())

    print("\n--- FTS5 ---")
    bench("FTS5 search", lambda: brain.execute(
        "SELECT d.id,rank FROM docs_fts f JOIN docs d ON d.id=f.rowid WHERE docs_fts MATCH ? LIMIT 10", (q,)).fetchall())

    print("\n--- Full hybrid (T1 hit) ---")
    bench("T1 pre-computed hybrid", lambda: pre.get(h))

    print("\n--- Full hybrid (T2: numpy+FTS5+RRF) ---")
    def full_hybrid():
        e = emb_dict[h]
        v = np_eng.search(e, 10)
        f = search_fts(brain, q, 10)
        hybrid_rrf(f, v, 5)
    bench("T2 numpy hybrid", full_hybrid)

    print("\n--- Embedding (cache hit) ---")
    bench("emb_cache SQLite lookup", lambda: cache.execute(
        "SELECT embedding FROM emb_cache WHERE query_hash=?", (h,)).fetchone())

    print("\n--- Embedding (in-memory dict) ---")
    bench("emb_dict[hash]", lambda: emb_dict.get(h))

    print("\n--- fastembed ONNX (warm, 8 threads) ---")
    bench("fastembed embed", lambda: embed_text(model, q), n=20)

    brain.close()
    cache.close()
    print(f"\n  Brain: {os.path.getsize(BRAIN_DB)/1024/1024:.1f}MB | "
          f"Cache: {os.path.getsize(CACHE_DB)/1024:.0f}KB | "
          f"Docs: {np_eng.doc_count}")

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# CLI
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def main():
    if len(sys.argv) < 2:
        print(__doc__.strip()); sys.exit(0)

    cmd = sys.argv[1]
    limit, port = 10, None
    args = sys.argv[2:]
    if "--limit" in args:
        i = args.index("--limit"); limit = int(args[i+1]); args = args[:i]+args[i+2:]
    if "--port" in args:
        i = args.index("--port"); port = int(args[i+1]); args = args[:i]+args[i+2:]

    cmds = {
        "find": lambda: cmd_find(args[0], limit) if args else None,
        "vec": lambda: cmd_vec(args[0], limit) if args else None,
        "hybrid": lambda: cmd_hybrid(args[0], limit) if args else None,
        "warm": cmd_warm,
        "stats": cmd_stats,
        "daemon": lambda: cmd_daemon(port),
        "bench": cmd_bench,
    }

    if cmd in cmds:
        cmds[cmd]()
    elif cmd in ("query", "q") and args:
        cmd_query_daemon(args[0], "hybrid", limit)
    else:
        print(__doc__.strip()); sys.exit(1)

if __name__ == "__main__":
    main()
