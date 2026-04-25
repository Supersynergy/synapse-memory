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
        self._has_vss = False
        # VSS extension can segfault on some builds — load in a safe way
        try:
            self.conn.execute("INSTALL vss;")
        except Exception:
            pass
        try:
            self.conn.execute("LOAD vss;")
            self._has_vss = True
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

    def _build_hnsw(self):
        if self._has_index or not self._has_vss:
            return {"hnsw_build_s": 0.0, "hnsw_skipped": True}
        # HNSW build in subprocess to avoid VSS SIGSEGV on mem.close() contaminating parent
        import subprocess, math
        try:
            n_full = self.conn.execute("SELECT count(*) FROM docs").fetchone()[0]
            print(f"  [duckdb] Timing HNSW build via subprocess (ef_construction=64, M=16)...", flush=True)
            script = (
                "import duckdb, time, json, sys\n"
                "conn = duckdb.connect(':memory:')\n"
                "try: conn.execute('LOAD vss;')\n"
                "except: pass\n"
                "conn.execute('CREATE TABLE docs (id VARCHAR, vec FLOAT[384])')\n"
                "conn.executemany('INSERT INTO docs VALUES (?, ?)', [[str(i), [float(j)/1000 for j in range(384)]] for i in range(1000)])\n"
                "t0 = time.perf_counter()\n"
                "conn.execute('CREATE INDEX hnsw_idx ON docs USING HNSW (vec) WITH (ef_construction=64, M=16)')\n"
                "print(time.perf_counter()-t0)\n"
            )
            proc = subprocess.run(
                [sys.executable, "-c", script],
                capture_output=True, text=True, timeout=60
            )
            if proc.returncode == 0 and proc.stdout.strip():
                build_s_1k = float(proc.stdout.strip())
                build_s_est = build_s_1k * (n_full / 1000) * math.log2(max(n_full, 2)) / math.log2(1000) if n_full > 1000 else build_s_1k
                print(f"  [duckdb] HNSW 1k-sample: {build_s_1k:.2f}s, est {build_s_est:.1f}s for {n_full} rows", flush=True)
                return {"hnsw_build_s_sample_1k": build_s_1k, "hnsw_build_s_estimated": build_s_est,
                        "hnsw_note": "1k synthetic sample via subprocess; persistence skipped (SIGSEGV on file-backed DB)"}
            else:
                print(f"  [duckdb] HNSW subprocess failed: {proc.stderr[:200]}", flush=True)
                return {"hnsw_build_s": 0.0, "hnsw_error": proc.stderr[:200]}
        except Exception as e:
            print(f"  [duckdb] HNSW timing failed: {e}", flush=True)
            return {"hnsw_build_s": 0.0, "hnsw_error": str(e)}

    def bulk_insert(self, docs):
        rows = [(d["id"], d["text"], d["category"], d["score"],
                 d["timestamp"], d["source"], d["lang"],
                 np.frombuffer(d["vec"], dtype=np.float32).tolist()) for d in docs]
        self.conn.executemany(
            "INSERT OR IGNORE INTO docs VALUES (?,?,?,?,?,?,?,?)", rows)
        self._inserted = getattr(self, "_inserted", 0) + len(rows)
        print(f"  [duckdb] inserted {self._inserted} rows total", flush=True)

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
        self._row_cache = {}  # id -> full row dict for O(1) lookup in update()

    def bulk_insert(self, docs):
        import pyarrow as pa
        _schema = pa.schema([
            pa.field("id", pa.string()),
            pa.field("text", pa.string()),
            pa.field("category", pa.string()),
            pa.field("score", pa.float64()),
            pa.field("timestamp", pa.int64()),
            pa.field("source", pa.string()),
            pa.field("lang", pa.string()),
            pa.field("vec", pa.list_(pa.float32(), 384)),
        ])
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
            self._tbl = self.db.create_table("docs", data=rows, schema=_schema, mode="overwrite")
        else:
            self._tbl.add(rows)
        for r in rows:
            self._row_cache[r["id"]] = r

    def update(self, ids, new_vecs, new_texts):
        import pyarrow as pa
        # Batch upsert via merge_insert — O(batch) not O(N) per row on append-log storage
        full_rows = []
        for doc_id, vec, text in zip(ids, new_vecs, new_texts):
            base = self._row_cache.get(doc_id, {})
            row = {
                "id": doc_id,
                "text": text,
                "category": base.get("category", ""),
                "score": base.get("score", 0.0),
                "timestamp": base.get("timestamp", 0),
                "source": base.get("source", ""),
                "lang": base.get("lang", ""),
                "vec": np.frombuffer(vec, dtype=np.float32).tolist(),
            }
            full_rows.append(row)
            self._row_cache[doc_id] = row
        updates_tbl = pa.Table.from_pylist(full_rows)
        (self._tbl.merge_insert("id")
            .when_matched_update_all()
            .when_not_matched_insert_all()
            .execute(updates_tbl))

    def hybrid_select(self, query_vec, text_filter, k=10):
        arr = np.frombuffer(query_vec, dtype=np.float32).tolist()
        try:
            rows = (self._tbl.search(arr, vector_column_name="vec")
                    .where(f"category = '{text_filter}'")
                    .limit(k)
                    .to_list())
        except Exception:
            rows = (self._tbl.search(arr, vector_column_name="vec").limit(k).to_list())
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

    def _ensure_server(self):
        import requests, subprocess, time as _time
        for attempt in range(3):
            try:
                r = requests.get(f"{self.base}/healthz", timeout=3)
                if r.status_code == 200:
                    return True
            except Exception:
                pass
            if attempt < 2:
                # Try to start qdrant — use QDRANT__STORAGE__STORAGE_PATH env var
                qdrant_bin = "/Users/master/.local/bin/qdrant"
                log_path = os.path.join(DIR, "qdrant.log")
                storage_dir = os.path.join(DIR, "_qdrant_storage")
                os.makedirs(storage_dir, exist_ok=True)
                env = os.environ.copy()
                env["QDRANT__STORAGE__STORAGE_PATH"] = storage_dir
                try:
                    subprocess.Popen(
                        [qdrant_bin],
                        stdout=open(log_path, "a"),
                        stderr=subprocess.STDOUT,
                        env=env,
                    )
                    _time.sleep(4)
                except Exception:
                    pass
        return False

    def setup(self, tmpdir):
        import requests
        self.base = "http://localhost:6333"
        self.collection = "bench_test"
        if not self._ensure_server():
            raise RuntimeError("Qdrant server unavailable at localhost:6333")
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

    def _int_to_str(self, i):
        for s, v in self._id_map.items():
            if v == i:
                return s
        return str(i)

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
        self._ensure_server()
        arr = np.frombuffer(query_vec, dtype=np.float32).tolist()
        r = requests.post(f"{self.base}/collections/{self.collection}/points/search",
                          json={
                              "vector": arr, "limit": k,
                              "filter": {"must": [{"key": "category",
                                                   "match": {"value": text_filter}}]},
                              "with_payload": False,
                          }, timeout=10)
        raw = r.json().get("result", [])
        # Map int ids back to original string ids for recall computation
        return [{"id": self._int_to_str(hit["id"]), "score": hit.get("score", 0)} for hit in raw]

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
    """Bulk insert all docs. HNSW build is measured AFTER timing as separate metric."""
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
    result = {"ops_sec": ops_sec, "elapsed_s": elapsed,
              "rss_mb": rss, "disk_mb": disk_mb,
              "cpu_pct_mean": float(np.mean(cpu_samples)) if cpu_samples else 0}
    # Build HNSW AFTER timing window — stored as separate metric
    if hasattr(adapter, "_build_hnsw"):
        result["hnsw"] = adapter._build_hnsw() or {}
    return result


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
    # Return updated id→vec_bytes so callers can sync their docs snapshot
    _updated = {doc_id: vec for doc_id, vec in zip(ids, new_vecs_bytes)}
    return {"ops_sec": ops_sec, "elapsed_s": elapsed,
            "rss_mb": rss,
            "cpu_pct_mean": float(np.mean(cpu_samples)) if cpu_samples else 0,
            "_updated_vecs": _updated}


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


ADAPTERS = {
    "sqlite-vec": SqliteVecAdapter,
    "duckdb": DuckDBAdapter,
    "lancedb": LanceDBAdapter,
    "qdrant": QdrantAdapter,
    "chromadb": ChromaAdapter,
    "synapse": SynapseAdapter,
}


# ─── Extended phases (--phases=all) ────────────────────────────────────────────

def run_phase_scale(adapter_cls, docs_full, scales=(1000, 10000, 100000)):
    """Scale-curve: insert+query at each scale, return list of {scale, insert_ops, query_ms}."""
    results = []
    for n in scales:
        if n > len(docs_full):
            continue
        subset = docs_full[:n]
        tmpdir = tempfile.mkdtemp(prefix=f"bench_scale_{adapter_cls.name}_{n}_")
        try:
            adapter = adapter_cls()
            adapter.setup(tmpdir)
            t0 = time.perf_counter()
            batch = 1000
            for i in range(0, len(subset), batch):
                adapter.bulk_insert(subset[i:i+batch])
            insert_s = time.perf_counter() - t0
            insert_ops = n / insert_s if insert_s > 0 else 0

            # 20 random queries
            rng = np.random.default_rng(42)
            lats = []
            for _ in range(20):
                q = rng.standard_normal(384).astype(np.float32)
                q /= np.linalg.norm(q)
                cat = random.choice(CATEGORIES)
                t0 = time.perf_counter()
                try:
                    adapter.hybrid_select(q.tobytes(), cat, k=10)
                except Exception:
                    pass
                lats.append((time.perf_counter() - t0) * 1000)
            adapter.teardown()
            results.append({"scale": n, "insert_ops_sec": insert_ops,
                            "query_p50_ms": sorted(lats)[10], "query_p95_ms": sorted(lats)[18]})
            print(f"    scale={n:>7d}  insert={insert_ops:.0f} ops/s  q_p50={sorted(lats)[10]:.1f}ms")
        except Exception as e:
            results.append({"scale": n, "error": str(e)})
            print(f"    scale={n}: ERROR {e}")
        finally:
            shutil.rmtree(tmpdir, ignore_errors=True)
    return results


def brute_force_recall(docs, query_vec, k=10, filter_cat=None):
    """Ground-truth top-k ids via cosine similarity (numpy)."""
    q = np.frombuffer(query_vec, dtype=np.float32)
    q = q / (np.linalg.norm(q) + 1e-9)
    filtered = [d for d in docs if filter_cat is None or d["category"] == filter_cat]
    if not filtered:
        return []
    vecs = np.array([np.frombuffer(d["vec"], dtype=np.float32) for d in filtered])
    norms = np.linalg.norm(vecs, axis=1, keepdims=True) + 1e-9
    vecs = vecs / norms
    scores = vecs @ q
    top_idx = np.argpartition(scores, -min(k, len(scores)))[-min(k, len(scores)):]
    top_idx = top_idx[np.argsort(scores[top_idx])[::-1]]
    return [filtered[i]["id"] for i in top_idx]


def run_phase_recall(adapter, docs, n_queries=50):
    """Recall@10 vs brute-force cosine ground truth."""
    rng = np.random.default_rng(123)
    hits = 0
    total = 0
    for _ in range(n_queries):
        q = rng.standard_normal(384).astype(np.float32)
        q /= np.linalg.norm(q)
        cat = random.choice(CATEGORIES)
        gt = set(brute_force_recall(docs, q.tobytes(), k=10, filter_cat=cat))
        try:
            result = adapter.hybrid_select(q.tobytes(), cat, k=10)
            # normalize result format
            if result and isinstance(result[0], (list, tuple)):
                got_ids = set(r[0] for r in result)
            elif result and isinstance(result[0], dict):
                got_ids = set(r.get("id", r.get("_id", "")) for r in result)
            else:
                got_ids = set(str(r) for r in result)
        except Exception:
            got_ids = set()
        hits += len(gt & got_ids)
        total += len(gt)
    recall = hits / total if total > 0 else 0.0
    return {"recall_at_10": recall, "n_queries": n_queries}


def run_phase_soak(adapter, docs, duration_s=180):
    """3-min sustained 80/20 — ops/sec drift + RSS growth (memory leak detector)."""
    rng = np.random.default_rng(77)
    write_docs = docs[:100]
    proc = psutil.Process(os.getpid())
    window = 10  # sample every 10s
    buckets = []  # (t, ops_in_window, rss_mb)
    ops_window = 0
    t_window_start = time.perf_counter()
    t_end = time.perf_counter() + duration_s

    while time.perf_counter() < t_end:
        if random.random() < 0.8:
            q = rng.standard_normal(384).astype(np.float32)
            q /= np.linalg.norm(q)
            cat = random.choice(CATEGORIES)
            try:
                adapter.hybrid_select(q.tobytes(), cat, k=10)
            except Exception:
                pass
        else:
            d = random.choice(write_docs)
            v = rng.standard_normal(384).astype(np.float32)
            v /= np.linalg.norm(v)
            try:
                adapter.update([d["id"]], [v.tobytes()], [f"soak_{ops_window}"])
            except Exception:
                pass
        ops_window += 1
        now = time.perf_counter()
        if now - t_window_start >= window:
            rss = proc.memory_info().rss / 1e6
            buckets.append({"t_s": now, "ops_per_s": ops_window / window, "rss_mb": rss})
            ops_window = 0
            t_window_start = now

    if not buckets:
        return {"error": "no data"}
    ops_first = buckets[0]["ops_per_s"]
    ops_last = buckets[-1]["ops_per_s"]
    rss_first = buckets[0]["rss_mb"]
    rss_last = buckets[-1]["rss_mb"]
    return {
        "duration_s": duration_s,
        "ops_first": ops_first,
        "ops_last": ops_last,
        "ops_drift_pct": ((ops_last - ops_first) / (ops_first + 1e-9)) * 100,
        "rss_first_mb": rss_first,
        "rss_last_mb": rss_last,
        "rss_growth_mb": rss_last - rss_first,
        "buckets": buckets,
    }


def run_phase_concurrency(adapter, docs, thread_counts=(1, 4, 8, 16), duration_s=10):
    """Concurrency sweep: ops/sec + p99 vs thread count."""
    results = []
    for n_threads in thread_counts:
        r = run_phase_d(adapter, docs, n_threads=n_threads, duration_s=duration_s)
        r["threads"] = n_threads
        results.append(r)
        print(f"    threads={n_threads:>2d}  ops/s={r['ops_sec']:.0f}  p99={r['p99_ms']:.1f}ms")
    return results


def run_phase_batch_update(adapter, docs, batch_sizes=(1, 100, 1000)):
    """Batch-size sweep for updates."""
    rng = np.random.default_rng(55)
    results = []
    sample_pool = docs[:2000]
    for bsz in batch_sizes:
        n_total = min(bsz * 10, 1000)
        sample = random.sample(sample_pool, min(n_total, len(sample_pool)))
        new_vecs = [rng.standard_normal(384).astype(np.float32) for _ in sample]
        for v in new_vecs:
            v /= np.linalg.norm(v)
        new_vecs_b = [v.tobytes() for v in new_vecs]
        new_texts = [f"batch_{i}" for i in range(len(sample))]
        ids = [d["id"] for d in sample]

        t0 = time.perf_counter()
        try:
            for i in range(0, len(ids), bsz):
                adapter.update(ids[i:i+bsz], new_vecs_b[i:i+bsz], new_texts[i:i+bsz])
            elapsed = time.perf_counter() - t0
            ops_sec = len(ids) / elapsed if elapsed > 0 else 0
        except Exception as e:
            ops_sec = 0
            elapsed = 0
        results.append({"batch_size": bsz, "ops_sec": ops_sec, "total": len(ids)})
        print(f"    batch_size={bsz:>5d}  ops/s={ops_sec:.0f}")
    return results


def run_adapter(adapter_cls, docs, dry_run=False, n_runs=3, warmup=1, phases="base", fast=False):
    tmpdir = tempfile.mkdtemp(prefix=f"bench_{adapter_cls.name}_")
    results = {"engine": adapter_cls.name, "n_docs": len(docs), "profile": "fast" if fast else "full"}

    fp = FAST_PROFILE if fast else {}
    _warmup = 0 if fast else warmup
    _n_update = fp.get("n_update", min(1000, len(docs))) if fast else min(1000, len(docs))
    _dur = fp.get("phase_c_duration", 5 if dry_run else 10) if fast else (5 if dry_run else 10)

    import signal

    class TimeoutError(Exception):
        pass

    def _timeout_handler(signum, frame):
        raise TimeoutError("engine timeout")

    try:
        adapter = adapter_cls()
        adapter.setup(tmpdir)

        # Warmup inserts
        for _ in range(_warmup):
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

        # Set per-engine timeout — duckdb gets 300s (INSERT with FLOAT[384] is ~10s/1k rows)
        if fast:
            engine_timeout = 300 if adapter_cls.name == "duckdb" else 90
        else:
            engine_timeout = 0
        if engine_timeout > 0:
            signal.alarm(0)  # cancel any leftover alarm first
            signal.signal(signal.SIGALRM, _timeout_handler)
            signal.alarm(engine_timeout)

        print(f"[engine={adapter_cls.name} phase=A scale={len(docs)}] starting")
        print(f"  [A] bulk_insert {len(docs)} docs...")
        phase_a = run_phase_a(adapter, docs)
        results["phase_a"] = phase_a
        print(f"      {phase_a['ops_sec']:.0f} ops/s  RSS={phase_a['rss_mb']:.0f}MB  disk={phase_a['disk_mb']:.1f}MB  cpu={phase_a['cpu_pct_mean']:.0f}%")
        print(f"[engine={adapter_cls.name} phase=A scale={len(docs)}] done ops_per_sec={phase_a['ops_sec']:.0f}")

        print(f"[engine={adapter_cls.name} phase=B scale={len(docs)}] starting")
        print(f"  [B] update {_n_update} docs...")
        phase_b = run_phase_b(adapter, docs, n_update=_n_update)
        # Sync docs snapshot so Phase E brute-force GT matches engine's current state
        _updated_vecs = phase_b.pop("_updated_vecs", {})
        if _updated_vecs:
            _docs_by_id = {d["id"]: d for d in docs}
            for _uid, _uvec in _updated_vecs.items():
                if _uid in _docs_by_id:
                    _docs_by_id[_uid]["vec"] = _uvec
        results["phase_b"] = phase_b
        print(f"      {phase_b['ops_sec']:.0f} ops/s  RSS={phase_b['rss_mb']:.0f}MB  cpu={phase_b['cpu_pct_mean']:.0f}%")
        print(f"[engine={adapter_cls.name} phase=B scale={len(docs)}] done ops_per_sec={phase_b['ops_sec']:.0f}")

        print(f"[engine={adapter_cls.name} phase=C scale={len(docs)}] starting")
        print(f"  [C] mixed 80/20 for {_dur}s...")
        phase_c = run_phase_c(adapter, docs, duration_s=_dur)
        results["phase_c"] = phase_c
        print(f"      {phase_c['ops_sec']:.1f} ops/s  p50={phase_c['p50_ms']:.1f}ms  p95={phase_c['p95_ms']:.1f}ms  p99={phase_c['p99_ms']:.1f}ms")
        print(f"[engine={adapter_cls.name} phase=C scale={len(docs)}] done ops_per_sec={phase_c['ops_sec']:.1f}")

        print(f"[engine={adapter_cls.name} phase=D scale={len(docs)}] starting")
        print(f"  [D] 8-thread concurrent select for {_dur}s...")
        phase_d = run_phase_d(adapter, docs, n_threads=8, duration_s=_dur)
        results["phase_d"] = phase_d
        print(f"      {phase_d['ops_sec']:.1f} ops/s  p99={phase_d['p99_ms']:.1f}ms")
        print(f"[engine={adapter_cls.name} phase=D scale={len(docs)}] done ops_per_sec={phase_d['ops_sec']:.1f}")

        # Cancel alarm before optional phases (brute-force recall can be slow)
        if engine_timeout > 0:
            signal.alarm(0)
            engine_timeout = 0

        if phases == "all" or fast:
            # E: recall@10
            n_recall = fp.get("phase_e_queries", 50) if fast else 50
            print(f"[engine={adapter_cls.name} phase=E scale={len(docs)}] starting")
            print(f"  [E] recall@10 ({n_recall} queries vs brute-force)...")
            phase_e = run_phase_recall(adapter, docs, n_queries=n_recall)
            results["phase_e"] = phase_e
            print(f"      recall@10={phase_e['recall_at_10']:.3f}")
            print(f"[engine={adapter_cls.name} phase=E scale={len(docs)}] done recall={phase_e['recall_at_10']:.3f}")

        if phases == "all":
            # F: concurrency sweep
            f_threads = fp.get("phase_f_threads", (8,)) if fast else (1, 4, 8, 16)
            print(f"[engine={adapter_cls.name} phase=F scale={len(docs)}] starting")
            print(f"  [F] concurrency sweep {f_threads} threads...")
            phase_f = run_phase_concurrency(adapter, docs, thread_counts=f_threads, duration_s=_dur)
            results["phase_f"] = phase_f
            print(f"[engine={adapter_cls.name} phase=F scale={len(docs)}] done")

            # G: batch update sweep
            g_batches = fp.get("phase_g_batches", (100,)) if fast else (1, 100, 1000)
            print(f"[engine={adapter_cls.name} phase=G scale={len(docs)}] starting")
            print(f"  [G] batch update sizes {g_batches}...")
            phase_g = run_phase_batch_update(adapter, docs, batch_sizes=g_batches)
            results["phase_g"] = phase_g
            print(f"[engine={adapter_cls.name} phase=G scale={len(docs)}] done")

            # H: soak — skip in fast mode and dry-run
            skip_soak = fast or dry_run
            if not skip_soak:
                print(f"[engine={adapter_cls.name} phase=H scale={len(docs)}] starting")
                print(f"  [H] soak test 180s mixed 80/20...")
                phase_h = run_phase_soak(adapter, docs, duration_s=180)
                results["phase_h"] = phase_h
                drift = phase_h.get("ops_drift_pct", 0)
                rss_growth = phase_h.get("rss_growth_mb", 0)
                print(f"      ops_drift={drift:.1f}%  rss_growth={rss_growth:.0f}MB")
                print(f"[engine={adapter_cls.name} phase=H scale={len(docs)}] done drift={drift:.1f}%")

        if engine_timeout > 0:
            signal.alarm(0)  # cancel alarm

        adapter.teardown()

    except Exception as e:
        try:
            if engine_timeout > 0:
                signal.alarm(0)
        except Exception:
            pass
        err_type = "TIMEOUT" if "timeout" in str(e).lower() else "ERROR"
        results["error"] = str(e)
        results["status"] = "SKIPPED" if err_type == "TIMEOUT" else "ERROR"
        print(f"  [{err_type}] {adapter_cls.name}: {e}")
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


FAST_PROFILE = {
    "n_docs": 10_000,
    "n_update": 500,
    "phase_c_duration": 5,
    "phase_d_duration": 5,
    "phase_e_queries": 50,
    "phase_f_threads": (8,),       # skip 1/4/16
    "phase_g_batches": (100,),     # skip 1/1000
    "skip_soak": True,
    "warmup": 0,                   # no warmup run
    "hnsw_ef_construction": 64,
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", default="all",
                        help="Comma-separated engines or 'all'")
    parser.add_argument("--dry-run", action="store_true",
                        help="Use 1k docs for validation")
    parser.add_argument("--n-docs", type=int, default=100000)
    parser.add_argument("--phases", default="base", choices=["base", "all"],
                        help="'base' = A/B/C/D only (compat). 'all' = +E/F/G/H (recall/concurrency/batch/soak)")
    parser.add_argument("--profile", default="full", choices=["full", "fast"],
                        help="'fast' = 10k docs, reduced phases, ≤5 min total")
    args = parser.parse_args()

    fast = args.profile == "fast"

    if fast:
        n_docs = FAST_PROFILE["n_docs"]
    elif args.dry_run:
        n_docs = 1000
    else:
        n_docs = args.n_docs
    print(f"[bench] Loading dataset ({n_docs} docs)...")
    docs = load_dataset(n_docs)
    print(f"[bench] Loaded {len(docs)} docs")

    if args.engine == "all":
        engines = list(ADAPTERS.keys())
    else:
        engines = [e.strip() for e in args.engine.split(",")]

    # Scale-curve: run separately before main phases if phases=all
    if args.phases == "all":
        print("\n[bench] === SCALE CURVE (1k/10k/100k) ===")
        for engine in engines:
            if engine not in ADAPTERS:
                continue
            print(f"  [{engine}]")
            try:
                scale_results = run_phase_scale(ADAPTERS[engine], docs,
                                                scales=(1000, 10000, min(100000, len(docs))))
                sc_path = os.path.join(RESULTS_DIR, f"{engine}_scale.jsonl")
                with open(sc_path, "w") as f:
                    f.write(json.dumps({"engine": engine, "scale_curve": scale_results}) + "\n")
            except Exception as e:
                print(f"    ERROR: {e}")

    all_results = []
    suffix = "fast" if fast else ("dry" if args.dry_run else "full")

    for engine in engines:
        if engine not in ADAPTERS:
            print(f"[warn] Unknown engine: {engine}, skipping")
            continue
        out_path = os.path.join(RESULTS_DIR, f"{engine}_{suffix}.jsonl")
        if fast and os.path.exists(out_path):
            print(f"\n[bench] === {engine} SKIPPED (result exists) ===")
            continue
        print(f"\n[bench] === {engine} ===")
        result = run_adapter(ADAPTERS[engine], docs, dry_run=args.dry_run, phases=args.phases,
                             fast=fast)
        all_results.append(result)

        out_path = os.path.join(RESULTS_DIR, f"{engine}_{suffix}.jsonl")
        with open(out_path, "w") as f:
            f.write(json.dumps(result) + "\n")
        print(f"  -> {out_path}")

    # Summary JSONL
    summary_path = os.path.join(RESULTS_DIR, f"summary_{suffix}.jsonl")
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
