#!/usr/bin/env python3
"""
Comprehensive benchmark: Synapse + 5 competitors
Phases: A=bulk insert, B=update, C=60s mixed 80/20, D=8-thread concurrent select
Metrics: ops/sec, p50/p95/p99 latency, peak RSS MB, CPU%
"""
import argparse, os, sys, time, json, random, threading, struct, gc, tempfile, shutil
import numpy as np
import pyarrow.parquet as pq
import psutil

DIR = os.path.dirname(os.path.abspath(__file__))
DATASET = os.path.join(DIR, "dataset.parquet")
RESULTS_DIR = os.path.join(DIR, "results")
os.makedirs(RESULTS_DIR, exist_ok=True)

# ─── CPU sampler ───────────────────────────────────────────────────────────────

class CPUSampler:
    def __init__(self, pid=None):
        self.proc = psutil.Process(pid or os.getpid())
        self._samples = []
        self._running = False
        self._thread = None

    def start(self):
        self._running = True
        self._samples = []
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self):
        while self._running:
            try:
                self._samples.append(self.proc.cpu_percent(interval=0.2))
            except Exception:
                break

    def stop(self):
        self._running = False
        if self._thread:
            self._thread.join(timeout=2)
        return self._samples

    def peak_rss_mb(self):
        try:
            return self.proc.memory_info().rss / 1e6
        except Exception:
            return 0.0


def percentiles(latencies, ps=(50, 95, 99)):
    if not latencies:
        return {p: 0.0 for p in ps}
    arr = sorted(latencies)
    n = len(arr)
    return {p: arr[int(n * p / 100)] * 1000 for p in ps}  # ms


# ─── Base adapter ──────────────────────────────────────────────────────────────

class Adapter:
    name = "base"

    def setup(self, tmpdir): pass
    def bulk_insert(self, docs): pass  # docs: list of dicts
    def update(self, ids, new_vecs, new_texts): pass
    def hybrid_select(self, query_vec, text_filter, k=10): pass
    def teardown(self): pass
    def disk_bytes(self): return 0


# ─── sqlite-vec adapter ────────────────────────────────────────────────────────

class SqliteVecAdapter(Adapter):
    name = "sqlite-vec"

    def setup(self, tmpdir):
        import sqlite3
        try:
            import sqlite_vec
        except ImportError:
            raise RuntimeError("sqlite-vec not installed")
        self.db_path = os.path.join(tmpdir, "sqlite_vec.db")
        self.conn = sqlite3.connect(self.db_path)
        self.conn.enable_load_extension(True)
        sqlite_vec.load(self.conn)
        self.conn.enable_load_extension(False)
        self.conn.execute("PRAGMA journal_mode=WAL")
        self.conn.execute("PRAGMA synchronous=NORMAL")
        self.conn.execute("PRAGMA cache_size=-65536")
        self.conn.execute("""
            CREATE TABLE IF NOT EXISTS docs (
                id TEXT PRIMARY KEY, text TEXT, category TEXT,
                score REAL, timestamp INTEGER, source TEXT, lang TEXT
            )
        """)
        self.conn.execute("""
            CREATE VIRTUAL TABLE IF NOT EXISTS vss USING vec0(
                id TEXT PRIMARY KEY, vec FLOAT[384]
            )
        """)
        self.conn.commit()

    def bulk_insert(self, docs):
        rows = [(d["id"], d["text"], d["category"], d["score"],
                 d["timestamp"], d["source"], d["lang"]) for d in docs]
        self.conn.executemany(
            "INSERT OR REPLACE INTO docs VALUES (?,?,?,?,?,?,?)", rows)
        vec_rows = [(d["id"], d["vec"]) for d in docs]
        # vec0 doesn't support INSERT OR REPLACE; delete then insert
        for row in vec_rows:
            self.conn.execute("DELETE FROM vss WHERE id=?", (row[0],))
        self.conn.executemany("INSERT INTO vss VALUES (?,?)", vec_rows)
        self.conn.commit()

    def update(self, ids, new_vecs, new_texts):
        for doc_id, vec, text in zip(ids, new_vecs, new_texts):
            self.conn.execute("UPDATE docs SET text=? WHERE id=?", (text, doc_id))
            self.conn.execute("DELETE FROM vss WHERE id=?", (doc_id,))
            self.conn.execute("INSERT INTO vss VALUES (?,?)", (doc_id, vec))
        self.conn.commit()

    def hybrid_select(self, query_vec, text_filter, k=10):
        rows = self.conn.execute("""
            SELECT v.id, v.distance FROM vss v
            JOIN docs d ON d.id = v.id
            WHERE v.vec MATCH ? AND k=? AND d.category = ?
            ORDER BY v.distance
        """, (query_vec, k, text_filter)).fetchall()
        return rows

    def disk_bytes(self):
        return os.path.getsize(self.db_path) if os.path.exists(self.db_path) else 0

    def teardown(self):
        self.conn.close()


# ─── DuckDB+VSS adapter ────────────────────────────────────────────────────────

class DuckDBAdapter(Adapter):
    name = "duckdb"

    def setup(self, tmpdir):
        import duckdb
        self.db_path = os.path.join(tmpdir, "duck.db")
        self.conn = duckdb.connect(self.db_path)
        try:
            self.conn.execute("INSTALL vss; LOAD vss;")
        except Exception:
            pass  # VSS may already be loaded
        try:
            self.conn.execute("LOAD vss;")
        except Exception:
            pass
        self.conn.execute("""
            CREATE TABLE IF NOT EXISTS docs (
                id VARCHAR PRIMARY KEY, text VARCHAR, category VARCHAR,
                score DOUBLE, timestamp BIGINT, source VARCHAR, lang VARCHAR,
                vec FLOAT[384]
            )
        """)
        # HNSW index created after bulk insert
        self._has_index = False

    def bulk_insert(self, docs):
        import duckdb
        rows = [(d["id"], d["text"], d["category"], d["score"],
                 d["timestamp"], d["source"], d["lang"],
                 np.frombuffer(d["vec"], dtype=np.float32).tolist()) for d in docs]
        self.conn.executemany(
            "INSERT OR REPLACE INTO docs VALUES (?,?,?,?,?,?,?,?)", rows)
        if not self._has_index:
            try:
                self.conn.execute(
                    "CREATE INDEX IF NOT EXISTS hnsw_idx ON docs USING HNSW (vec)")
                self._has_index = True
            except Exception:
                pass

    def update(self, ids, new_vecs, new_texts):
        for doc_id, vec, text in zip(ids, new_vecs, new_texts):
            arr = np.frombuffer(vec, dtype=np.float32).tolist()
            self.conn.execute(
                "UPDATE docs SET text=?, vec=? WHERE id=?", (text, arr, doc_id))

    def hybrid_select(self, query_vec, text_filter, k=10):
        arr = np.frombuffer(query_vec, dtype=np.float32).tolist()
        try:
            rows = self.conn.execute("""
                SELECT id, array_cosine_similarity(vec, ?::FLOAT[384]) AS score
                FROM docs WHERE category = ?
                ORDER BY score DESC LIMIT ?
            """, (arr, text_filter, k)).fetchall()
        except Exception:
            rows = self.conn.execute("""
                SELECT id FROM docs WHERE category = ? LIMIT ?
            """, (text_filter, k)).fetchall()
        return rows

    def disk_bytes(self):
        return os.path.getsize(self.db_path) if os.path.exists(self.db_path) else 0

    def teardown(self):
        self.conn.close()


# ─── LanceDB adapter ──────────────────────────────────────────────────────────

class LanceDBAdapter(Adapter):
    name = "lancedb"

    def setup(self, tmpdir):
        import lancedb
        self.db_path = os.path.join(tmpdir, "lance")
        self.db = lancedb.connect(self.db_path)
        self._tbl = None

    def bulk_insert(self, docs):
        import pyarrow as pa
        rows = [{
            "id": d["id"],
            "text": d["text"],
            "category": d["category"],
            "score": float(d["score"]),
            "timestamp": int(d["timestamp"]),
            "source": d["source"],
            "lang": d["lang"],
            "vec": np.frombuffer(d["vec"], dtype=np.float32).tolist(),
        } for d in docs]
        if self._tbl is None:
            self._tbl = self.db.create_table("docs", data=rows, mode="overwrite")
        else:
            self._tbl.add(rows)

    def update(self, ids, new_vecs, new_texts):
        for doc_id, vec, text in zip(ids, new_vecs, new_texts):
            arr = np.frombuffer(vec, dtype=np.float32).tolist()
            self._tbl.update(where=f"id = '{doc_id}'",
                             values={"vec": arr, "text": text})

    def hybrid_select(self, query_vec, text_filter, k=10):
        arr = np.frombuffer(query_vec, dtype=np.float32).tolist()
        try:
            rows = (self._tbl.search(arr)
                    .where(f"category = '{text_filter}'")
                    .limit(k)
                    .to_list())
        except Exception:
            rows = (self._tbl.search(arr).limit(k).to_list())
        return rows

    def disk_bytes(self):
        total = 0
        for root, dirs, files in os.walk(self.db_path):
            total += sum(os.path.getsize(os.path.join(root, f)) for f in files)
        return total

    def teardown(self):
        pass


# ─── Qdrant adapter ───────────────────────────────────────────────────────────

class QdrantAdapter(Adapter):
    name = "qdrant"

    def setup(self, tmpdir):
        import requests
        self.base = "http://localhost:6333"
        self.collection = "bench_test"
        # Delete if exists
        try:
            requests.delete(f"{self.base}/collections/{self.collection}", timeout=5)
        except Exception:
            pass
        r = requests.put(f"{self.base}/collections/{self.collection}", json={
            "vectors": {"size": 384, "distance": "Cosine"}
        }, timeout=10)
        if r.status_code not in (200, 201):
            raise RuntimeError(f"Qdrant create collection failed: {r.text}")
        self._id_map = {}  # str id -> int
        self._counter = 0

    def _str_to_int(self, s):
        if s not in self._id_map:
            self._id_map[s] = self._counter
            self._counter += 1
        return self._id_map[s]

    def bulk_insert(self, docs):
        import requests
        batch_size = 500
        for i in range(0, len(docs), batch_size):
            chunk = docs[i:i+batch_size]
            points = [{
                "id": self._str_to_int(d["id"]),
                "vector": np.frombuffer(d["vec"], dtype=np.float32).tolist(),
                "payload": {
                    "id": d["id"], "text": d["text"][:200],
                    "category": d["category"], "score": d["score"],
                }
            } for d in chunk]
            r = requests.put(f"{self.base}/collections/{self.collection}/points",
                             json={"points": points}, timeout=30)
            if r.status_code not in (200, 201):
                raise RuntimeError(f"Qdrant insert failed: {r.text[:200]}")

    def update(self, ids, new_vecs, new_texts):
        import requests
        points = [{
            "id": self._str_to_int(doc_id),
            "vector": np.frombuffer(vec, dtype=np.float32).tolist(),
            "payload": {"text": text[:200]}
        } for doc_id, vec, text in zip(ids, new_vecs, new_texts)]
        r = requests.put(f"{self.base}/collections/{self.collection}/points",
                         json={"points": points}, timeout=30)
        if r.status_code not in (200, 201):
            raise RuntimeError(f"Qdrant update failed: {r.text[:200]}")

    def hybrid_select(self, query_vec, text_filter, k=10):
        import requests
        arr = np.frombuffer(query_vec, dtype=np.float32).tolist()
        r = requests.post(f"{self.base}/collections/{self.collection}/points/search",
                          json={
                              "vector": arr, "limit": k,
                              "filter": {"must": [{"key": "category",
                                                   "match": {"value": text_filter}}]},
                              "with_payload": False,
                          }, timeout=10)
        return r.json().get("result", [])

    def disk_bytes(self):
        return 0  # external process

    def teardown(self):
        import requests
        try:
            requests.delete(f"http://localhost:6333/collections/{self.collection}", timeout=5)
        except Exception:
            pass


# ─── ChromaDB adapter ─────────────────────────────────────────────────────────

class ChromaAdapter(Adapter):
    name = "chromadb"

    def setup(self, tmpdir):
        import chromadb
        self.db_path = os.path.join(tmpdir, "chroma")
        self.client = chromadb.PersistentClient(path=self.db_path)
        try:
            self.client.delete_collection("bench")
        except Exception:
            pass
        self.col = self.client.create_collection("bench",
            metadata={"hnsw:space": "cosine"})

    def bulk_insert(self, docs):
        batch_size = 500
        for i in range(0, len(docs), batch_size):
            chunk = docs[i:i+batch_size]
            self.col.upsert(
                ids=[d["id"] for d in chunk],
                embeddings=[np.frombuffer(d["vec"], dtype=np.float32).tolist() for d in chunk],
                documents=[d["text"][:500] for d in chunk],
                metadatas=[{"category": d["category"], "score": d["score"]} for d in chunk],
            )

    def update(self, ids, new_vecs, new_texts):
        self.col.update(
            ids=ids,
            embeddings=[np.frombuffer(v, dtype=np.float32).tolist() for v in new_vecs],
            documents=[t[:500] for t in new_texts],
        )

    def hybrid_select(self, query_vec, text_filter, k=10):
        arr = np.frombuffer(query_vec, dtype=np.float32).tolist()
        r = self.col.query(
            query_embeddings=[arr], n_results=k,
            where={"category": {"$eq": text_filter}},
        )
        return r["ids"][0] if r["ids"] else []

    def disk_bytes(self):
        total = 0
        for root, dirs, files in os.walk(self.db_path):
            total += sum(os.path.getsize(os.path.join(root, f)) for f in files)
        return total

    def teardown(self):
        pass


# ─── Synapse adapter ──────────────────────────────────────────────────────────

class SynapseAdapter(Adapter):
    name = "synapse"

    def setup(self, tmpdir):
        # Try turbo daemon first
        import requests
        self._use_daemon = False
        try:
            r = requests.get("http://localhost:9477/health", timeout=2)
            if r.status_code == 200:
                self._use_daemon = True
                print("[synapse] Using turbo daemon at :9477")
        except Exception:
            pass

        if not self._use_daemon:
            print("[synapse] Daemon unavailable, falling back to direct sqlite-vec")
            import sqlite3
            try:
                import sqlite_vec
            except ImportError:
                raise RuntimeError("sqlite-vec not available for Synapse fallback")
            self.db_path = os.path.join(tmpdir, "synapse_bench.db")
            self.conn = sqlite3.connect(self.db_path)
            self.conn.enable_load_extension(True)
            sqlite_vec.load(self.conn)
            self.conn.enable_load_extension(False)
            self.conn.execute("PRAGMA journal_mode=WAL")
            self.conn.execute("PRAGMA synchronous=NORMAL")
            self.conn.execute("PRAGMA cache_size=-131072")
            self.conn.execute("""
                CREATE TABLE IF NOT EXISTS docs (
                    id TEXT PRIMARY KEY, text TEXT, category TEXT,
                    score REAL, timestamp INTEGER, source TEXT, lang TEXT
                )
            """)
            self.conn.execute("""
                CREATE VIRTUAL TABLE IF NOT EXISTS vss USING vec0(
                    id TEXT PRIMARY KEY, vec FLOAT[384]
                )
            """)
            self.conn.commit()

    def bulk_insert(self, docs):
        if self._use_daemon:
            import requests
            batch_size = 100
            for i in range(0, len(docs), batch_size):
                chunk = docs[i:i+batch_size]
                payload = [{"id": d["id"], "text": d["text"],
                            "vec": np.frombuffer(d["vec"], dtype=np.float32).tolist()} for d in chunk]
                requests.post("http://localhost:9477/insert_batch", json=payload, timeout=30)
        else:
            rows = [(d["id"], d["text"], d["category"], d["score"],
                     d["timestamp"], d["source"], d["lang"]) for d in docs]
            self.conn.executemany("INSERT OR REPLACE INTO docs VALUES (?,?,?,?,?,?,?)", rows)
            vec_rows = [(d["id"], d["vec"]) for d in docs]
            for row in vec_rows:
                self.conn.execute("DELETE FROM vss WHERE id=?", (row[0],))
            self.conn.executemany("INSERT INTO vss VALUES (?,?)", vec_rows)
            self.conn.commit()

    def update(self, ids, new_vecs, new_texts):
        if self._use_daemon:
            import requests
            payload = [{"id": doc_id, "text": text,
                        "vec": np.frombuffer(vec, dtype=np.float32).tolist()}
                       for doc_id, vec, text in zip(ids, new_vecs, new_texts)]
            requests.post("http://localhost:9477/insert_batch", json=payload, timeout=30)
        else:
            for doc_id, vec, text in zip(ids, new_vecs, new_texts):
                self.conn.execute("UPDATE docs SET text=? WHERE id=?", (text, doc_id))
                self.conn.execute("DELETE FROM vss WHERE id=?", (doc_id,))
                self.conn.execute("INSERT INTO vss VALUES (?,?)", (doc_id, vec))
            self.conn.commit()

    def hybrid_select(self, query_vec, text_filter, k=10):
        if self._use_daemon:
            import requests
            arr = np.frombuffer(query_vec, dtype=np.float32).tolist()
            r = requests.post("http://localhost:9477/query",
                              json={"vec": arr, "text": text_filter, "k": k}, timeout=5)
            return r.json().get("results", [])
        else:
            rows = self.conn.execute("""
                SELECT v.id, v.distance FROM vss v
                JOIN docs d ON d.id = v.id
                WHERE v.vec MATCH ? AND k=? AND d.category = ?
                ORDER BY v.distance
            """, (query_vec, k, text_filter)).fetchall()
            return rows

    def disk_bytes(self):
        if not self._use_daemon and hasattr(self, "db_path"):
            return os.path.getsize(self.db_path) if os.path.exists(self.db_path) else 0
        return 0

    def teardown(self):
        if not self._use_daemon and hasattr(self, "conn"):
            self.conn.close()


# ─── Benchmark runner ─────────────────────────────────────────────────────────

CATEGORIES = ["tech", "science", "news", "docs", "forum", "blog", "paper", "wiki"]

def load_dataset(n=None):
    tbl = pq.read_table(DATASET)
    if n:
        tbl = tbl.slice(0, n)
    docs = []
    for i in range(len(tbl)):
        docs.append({
            "id": tbl["id"][i].as_py(),
            "text": tbl["text"][i].as_py(),
            "vec": tbl["vec"][i].as_py(),
            "category": tbl["category"][i].as_py(),
            "score": tbl["score"][i].as_py(),
            "timestamp": tbl["timestamp"][i].as_py(),
            "source": tbl["source"][i].as_py(),
            "lang": tbl["lang"][i].as_py(),
        })
    return docs


def run_phase_a(adapter, docs):
    """Bulk insert all docs, measure throughput."""
    sampler = CPUSampler()
    sampler.start()
    t0 = time.perf_counter()
    batch_size = 1000
    for i in range(0, len(docs), batch_size):
        adapter.bulk_insert(docs[i:i+batch_size])
    elapsed = time.perf_counter() - t0
    cpu_samples = sampler.stop()
    rss = sampler.peak_rss_mb()
    ops_sec = len(docs) / elapsed if elapsed > 0 else 0
    disk_mb = adapter.disk_bytes() / 1e6
    return {"ops_sec": ops_sec, "elapsed_s": elapsed,
            "rss_mb": rss, "disk_mb": disk_mb,
            "cpu_pct_mean": float(np.mean(cpu_samples)) if cpu_samples else 0}


def run_phase_b(adapter, docs, n_update=1000):
    """Update random subset."""
    rng = np.random.default_rng(99)
    sample = random.sample(docs, min(n_update, len(docs)))
    new_vecs = [rng.standard_normal(384).astype(np.float32) for _ in sample]
    for v in new_vecs:
        v /= np.linalg.norm(v)
    new_vecs_bytes = [v.tobytes() for v in new_vecs]
    new_texts = [f"updated_{d['id']}" for d in sample]
    ids = [d["id"] for d in sample]

    sampler = CPUSampler()
    sampler.start()
    t0 = time.perf_counter()
    batch_size = 100
    for i in range(0, len(ids), batch_size):
        adapter.update(ids[i:i+batch_size],
                       new_vecs_bytes[i:i+batch_size],
                       new_texts[i:i+batch_size])
    elapsed = time.perf_counter() - t0
    cpu_samples = sampler.stop()
    rss = sampler.peak_rss_mb()
    ops_sec = len(ids) / elapsed if elapsed > 0 else 0
    return {"ops_sec": ops_sec, "elapsed_s": elapsed,
            "rss_mb": rss,
            "cpu_pct_mean": float(np.mean(cpu_samples)) if cpu_samples else 0}


def run_phase_c(adapter, docs, duration_s=10):
    """Mixed 80/20 read/write workload for duration_s seconds."""
    rng = np.random.default_rng(77)
    latencies = []
    ops = 0
    t_end = time.perf_counter() + duration_s
    write_docs = docs[:100]  # small pool for writes

    while time.perf_counter() < t_end:
        t0 = time.perf_counter()
        if random.random() < 0.8:
            # read
            qvec = rng.standard_normal(384).astype(np.float32)
            qvec /= np.linalg.norm(qvec)
            cat = random.choice(CATEGORIES)
            try:
                adapter.hybrid_select(qvec.tobytes(), cat, k=10)
            except Exception:
                pass
        else:
            # write (update 1 doc)
            d = random.choice(write_docs)
            v = rng.standard_normal(384).astype(np.float32)
            v /= np.linalg.norm(v)
            try:
                adapter.update([d["id"]], [v.tobytes()], [f"mixed_{ops}"])
            except Exception:
                pass
        latencies.append(time.perf_counter() - t0)
        ops += 1

    pct = percentiles(latencies)
    return {"ops_sec": ops / duration_s, "total_ops": ops,
            "p50_ms": pct[50], "p95_ms": pct[95], "p99_ms": pct[99]}


def run_phase_d(adapter, docs, n_threads=8, duration_s=10):
    """Concurrent select from N threads."""
    rng = np.random.default_rng(55)
    all_latencies = []
    lock = threading.Lock()
    stop_event = threading.Event()

    def worker():
        local_rng = np.random.default_rng(random.randint(0, 999999))
        while not stop_event.is_set():
            qvec = local_rng.standard_normal(384).astype(np.float32)
            qvec /= np.linalg.norm(qvec)
            cat = random.choice(CATEGORIES)
            t0 = time.perf_counter()
            try:
                adapter.hybrid_select(qvec.tobytes(), cat, k=10)
            except Exception:
                pass
            lat = time.perf_counter() - t0
            with lock:
                all_latencies.append(lat)

    threads = [threading.Thread(target=worker, daemon=True) for _ in range(n_threads)]
    for t in threads:
        t.start()
    time.sleep(duration_s)
    stop_event.set()
    for t in threads:
        t.join(timeout=5)

    pct = percentiles(all_latencies)
    total = len(all_latencies)
    return {"ops_sec": total / duration_s, "total_ops": total,
            "p50_ms": pct[50], "p95_ms": pct[95], "p99_ms": pct[99],
            "threads": n_threads}


def run_adapter(adapter_cls, docs, dry_run=False, n_runs=3, warmup=1):
    tmpdir = tempfile.mkdtemp(prefix=f"bench_{adapter_cls.name}_")
    results = {"engine": adapter_cls.name, "n_docs": len(docs)}

    try:
        adapter = adapter_cls()
        adapter.setup(tmpdir)

        # Warmup inserts (small, separate tmpdir)
        for _ in range(warmup):
            wtmp = tempfile.mkdtemp(prefix=f"bench_warm_{adapter_cls.name}_")
            try:
                wadapter = adapter_cls()
                wadapter.setup(wtmp)
                wadapter.bulk_insert(docs[:min(50, len(docs))])
                wadapter.teardown()
            except Exception:
                pass
            finally:
                shutil.rmtree(wtmp, ignore_errors=True)

        print(f"  [A] bulk_insert {len(docs)} docs...")
        phase_a = run_phase_a(adapter, docs)
        results["phase_a"] = phase_a
        print(f"      {phase_a['ops_sec']:.0f} ops/s  RSS={phase_a['rss_mb']:.0f}MB  disk={phase_a['disk_mb']:.1f}MB  cpu={phase_a['cpu_pct_mean']:.0f}%")

        print(f"  [B] update {min(1000, len(docs))} docs...")
        phase_b = run_phase_b(adapter, docs, n_update=min(1000, len(docs)))
        results["phase_b"] = phase_b
        print(f"      {phase_b['ops_sec']:.0f} ops/s  RSS={phase_b['rss_mb']:.0f}MB  cpu={phase_b['cpu_pct_mean']:.0f}%")

        dur = 5 if dry_run else 10
        print(f"  [C] mixed 80/20 for {dur}s...")
        phase_c = run_phase_c(adapter, docs, duration_s=dur)
        results["phase_c"] = phase_c
        print(f"      {phase_c['ops_sec']:.1f} ops/s  p50={phase_c['p50_ms']:.1f}ms  p95={phase_c['p95_ms']:.1f}ms  p99={phase_c['p99_ms']:.1f}ms")

        print(f"  [D] 8-thread concurrent select for {dur}s...")
        phase_d = run_phase_d(adapter, docs, n_threads=8, duration_s=dur)
        results["phase_d"] = phase_d
        print(f"      {phase_d['ops_sec']:.1f} ops/s  p99={phase_d['p99_ms']:.1f}ms")

        adapter.teardown()

    except Exception as e:
        results["error"] = str(e)
        print(f"  [ERROR] {adapter_cls.name}: {e}")
    finally:
        try:
            shutil.rmtree(tmpdir, ignore_errors=True)
        except Exception:
            pass

    return results


ADAPTERS = {
    "sqlite-vec": SqliteVecAdapter,
    "duckdb": DuckDBAdapter,
    "lancedb": LanceDBAdapter,
    "qdrant": QdrantAdapter,
    "chromadb": ChromaAdapter,
    "synapse": SynapseAdapter,
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", default="all",
                        help="Comma-separated engines or 'all'")
    parser.add_argument("--dry-run", action="store_true",
                        help="Use 1k docs for validation")
    parser.add_argument("--n-docs", type=int, default=100000)
    args = parser.parse_args()

    n_docs = 1000 if args.dry_run else args.n_docs
    print(f"[bench] Loading dataset ({n_docs} docs)...")
    docs = load_dataset(n_docs)
    print(f"[bench] Loaded {len(docs)} docs")

    if args.engine == "all":
        engines = list(ADAPTERS.keys())
    else:
        engines = [e.strip() for e in args.engine.split(",")]

    all_results = []
    for engine in engines:
        if engine not in ADAPTERS:
            print(f"[warn] Unknown engine: {engine}, skipping")
            continue
        print(f"\n[bench] === {engine} ===")
        result = run_adapter(ADAPTERS[engine], docs, dry_run=args.dry_run)
        all_results.append(result)

        suffix = "dry" if args.dry_run else "full"
        out_path = os.path.join(RESULTS_DIR, f"{engine}_{suffix}.jsonl")
        with open(out_path, "a") as f:
            f.write(json.dumps(result) + "\n")
        print(f"  -> {out_path}")

    # Summary JSONL
    summary_path = os.path.join(RESULTS_DIR, f"summary_{'dry' if args.dry_run else 'full'}.jsonl")
    with open(summary_path, "w") as f:
        for r in all_results:
            f.write(json.dumps(r) + "\n")

    print(f"\n[bench] Results: {summary_path}")
    print("\n=== QUICK SUMMARY ===")
    print(f"{'Engine':<14} {'InsertOps/s':>12} {'UpdateOps/s':>12} {'MixedOps/s':>12} {'p95ms':>8} {'RSS MB':>8} {'Disk MB':>8}")
    print("-" * 80)
    for r in all_results:
        if "error" in r and "phase_a" not in r:
            print(f"{r['engine']:<14} ERROR: {r['error'][:40]}")
            continue
        a = r.get("phase_a", {})
        b = r.get("phase_b", {})
        c = r.get("phase_c", {})
        print(f"{r['engine']:<14} {a.get('ops_sec',0):>12.0f} {b.get('ops_sec',0):>12.0f} "
              f"{c.get('ops_sec',0):>12.1f} {c.get('p95_ms',0):>8.1f} "
              f"{a.get('rss_mb',0):>8.0f} {a.get('disk_mb',0):>8.1f}")


if __name__ == "__main__":
    main()
