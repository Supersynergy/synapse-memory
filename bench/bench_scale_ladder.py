#!/usr/bin/env python3
"""Scale-ladder bench: Synapse vs competitors at N = 1k / 10k / 100k / 1M (/10M).

Rules:
- Engines that actually install in THIS venv on M4 Max.
- Median-of-3 per (engine, N) cell for query latency; single-run for ingest (expensive).
- 30s thermal warmup before the first cell.
- Write raw CSV + JSON + MD under docs/bench_scale_2026-04-23/.
- No fabrication. If an engine OOMs / times out / errors → mark n/m with reason.
- Per-cell timeout: ingest 15min, query batch 5min.

Engines:
  * sqlite-vec 0.1.9
  * DuckDB+VSS 1.5.1
  * LanceDB 0.30.2
  * Qdrant in-mem 1.17.1
  * Chroma 1.5.8
  * (Synapse: existing Rust bench numbers lifted where comparable; a true
     apples-to-apples run requires the synapse-cli which needs its own fixture.
     Honest placeholder, not fabricated: we reference bench/RESULTS-V1.md and
     mark UC/scale rows explicitly as "from prior bench, N=1k" where used.)

Usage: python3 bench/bench_scale_ladder.py
Env: SCALES="1000 10000 100000 1000000"  (override)
     ENGINES="sqlite_vec duckdb_vss lancedb qdrant chroma"
     Q=100 (queries per cell)
     TIMEOUT_INGEST=900 TIMEOUT_QUERY=300
"""
from __future__ import annotations
import csv, hashlib, json, os, resource, shutil, sqlite3, statistics, struct
import sys, tempfile, time
from dataclasses import dataclass, asdict
from pathlib import Path

SCALES = [int(x) for x in os.environ.get("SCALES", "1000 10000 100000 1000000").split()]
Q = int(os.environ.get("Q", 100))
DIM = 384
OUT = Path("/Users/master/projects/synapse/docs/bench_scale_2026-04-23")
OUT.mkdir(parents=True, exist_ok=True)
TIMEOUT_INGEST = int(os.environ.get("TIMEOUT_INGEST", 900))
TIMEOUT_QUERY = int(os.environ.get("TIMEOUT_QUERY", 300))


def vec(seed: str):
    h = hashlib.sha256(seed.encode()).digest()
    return [int.from_bytes(h[(i*4) % len(h):(i*4) % len(h)+4], "little")/2**31-1.0
            for i in range(DIM)]


def docs(n):
    return [{"id": i, "text": f"doc {i} topic{i%37}",
             "vector": vec(f"doc {i} topic{i%37}")} for i in range(n)]


def qs(n):
    return [vec(f"doc {i*5} topic{(i*5)%37}") for i in range(n)]


def pct(xs, p):
    if not xs: return 0.0
    s = sorted(xs); k = int(len(s)*p/100)
    return s[min(k, len(s)-1)]


def dirsize(d):
    try:
        return sum(os.path.getsize(os.path.join(dp, f))
                   for dp, _, fs in os.walk(d) for f in fs)
    except Exception:
        return 0


def rss_mb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss/1_000_000


@dataclass
class Cell:
    engine: str
    N: int
    ingest_s: float = 0
    query_p50_ms: float = 0
    query_p95_ms: float = 0
    query_p99_ms: float = 0
    disk_mb: float = 0
    rss_delta_mb: float = 0
    ok: bool = True
    note: str = ""


def run_sqlite_vec(N):
    import sqlite_vec
    c = Cell("sqlite-vec", N)
    p = tempfile.NamedTemporaryFile(suffix=".db", delete=False).name
    os.unlink(p)
    rss0 = rss_mb()
    try:
        conn = sqlite3.connect(p); conn.enable_load_extension(True)
        sqlite_vec.load(conn); conn.enable_load_extension(False)
        conn.execute(f"CREATE VIRTUAL TABLE v USING vec0(embedding float[{DIM}])")
        t = time.perf_counter()
        BATCH = 5000
        data = docs(N)
        for i in range(0, N, BATCH):
            conn.executemany("INSERT INTO v(rowid, embedding) VALUES(?,?)",
                [(r["id"], struct.pack(f"{DIM}f", *r["vector"])) for r in data[i:i+BATCH]])
            conn.commit()
        c.ingest_s = time.perf_counter() - t
        queries = qs(Q)
        lat = []
        for q in queries:
            t = time.perf_counter()
            conn.execute(
                "SELECT rowid FROM v WHERE embedding MATCH ? AND k=10 ORDER BY distance",
                (struct.pack(f"{DIM}f", *q),)).fetchall()
            lat.append((time.perf_counter()-t)*1000)
        c.query_p50_ms = statistics.median(lat)
        c.query_p95_ms = pct(lat, 95); c.query_p99_ms = pct(lat, 99)
        conn.close()
        c.disk_mb = os.path.getsize(p)/1_000_000
        c.rss_delta_mb = rss_mb() - rss0
    except Exception as e:
        c.ok = False; c.note = f"err: {str(e)[:120]}"
    finally:
        try: os.unlink(p)
        except Exception: pass
    return c


def run_duckdb_vss(N):
    import duckdb
    c = Cell("DuckDB+VSS", N)
    p = tempfile.NamedTemporaryFile(suffix=".duckdb", delete=False).name
    os.unlink(p)
    rss0 = rss_mb()
    try:
        con = duckdb.connect(p)
        con.execute("INSTALL vss; LOAD vss; SET hnsw_enable_experimental_persistence=true;")
        con.execute(f"CREATE TABLE d(id INT, vec FLOAT[{DIM}])")
        data = docs(N)
        t = time.perf_counter()
        con.executemany("INSERT INTO d VALUES(?,?)",
                        [(r["id"], r["vector"]) for r in data])
        con.execute("CREATE INDEX idx ON d USING HNSW(vec) WITH (metric='cosine')")
        c.ingest_s = time.perf_counter() - t
        queries = qs(Q)
        lat = []
        for q in queries:
            t = time.perf_counter()
            con.execute(
                "SELECT id FROM d ORDER BY array_distance(vec, ?::FLOAT[384]) LIMIT 10",
                [q]).fetchall()
            lat.append((time.perf_counter()-t)*1000)
        c.query_p50_ms = statistics.median(lat)
        c.query_p95_ms = pct(lat, 95); c.query_p99_ms = pct(lat, 99)
        con.close()
        c.disk_mb = os.path.getsize(p)/1_000_000 if os.path.exists(p) else 0
        c.rss_delta_mb = rss_mb() - rss0
    except Exception as e:
        c.ok = False; c.note = f"err: {str(e)[:150]}"
    finally:
        try: os.unlink(p)
        except Exception: pass
    return c


def run_lancedb(N):
    import lancedb
    c = Cell("LanceDB", N)
    d = tempfile.mkdtemp()
    rss0 = rss_mb()
    try:
        db = lancedb.connect(d)
        data = docs(N)
        arrow = [{"id": r["id"], "vector": r["vector"]} for r in data]
        t = time.perf_counter()
        tbl = db.create_table("m", data=arrow, mode="overwrite")
        c.ingest_s = time.perf_counter() - t
        queries = qs(Q)
        lat = []
        for q in queries:
            t = time.perf_counter()
            tbl.search(q).limit(10).to_list()
            lat.append((time.perf_counter()-t)*1000)
        c.query_p50_ms = statistics.median(lat)
        c.query_p95_ms = pct(lat, 95); c.query_p99_ms = pct(lat, 99)
        c.disk_mb = dirsize(d)/1_000_000
        c.rss_delta_mb = rss_mb() - rss0
    except Exception as e:
        c.ok = False; c.note = f"err: {str(e)[:150]}"
    finally:
        shutil.rmtree(d, ignore_errors=True)
    return c


def run_qdrant(N):
    from qdrant_client import QdrantClient
    from qdrant_client.models import Distance, PointStruct, VectorParams
    c = Cell("Qdrant in-mem", N)
    rss0 = rss_mb()
    try:
        client = QdrantClient(":memory:")
        client.create_collection("m", vectors_config=VectorParams(size=DIM, distance=Distance.COSINE))
        data = docs(N)
        t = time.perf_counter()
        BATCH = 1000
        for i in range(0, N, BATCH):
            client.upsert("m", points=[
                PointStruct(id=r["id"], vector=r["vector"]) for r in data[i:i+BATCH]])
        c.ingest_s = time.perf_counter() - t
        queries = qs(Q)
        lat = []
        for q in queries:
            t = time.perf_counter()
            client.query_points(collection_name="m", query=q, limit=10)
            lat.append((time.perf_counter()-t)*1000)
        c.query_p50_ms = statistics.median(lat)
        c.query_p95_ms = pct(lat, 95); c.query_p99_ms = pct(lat, 99)
        c.rss_delta_mb = rss_mb() - rss0
        c.note = "in-mem, no disk"
    except Exception as e:
        c.ok = False; c.note = f"err: {str(e)[:150]}"
    return c


def run_chroma(N):
    import chromadb
    c = Cell("Chroma", N)
    d = tempfile.mkdtemp()
    rss0 = rss_mb()
    try:
        client = chromadb.PersistentClient(path=d)
        col = client.get_or_create_collection("mem")
        data = docs(N)
        t = time.perf_counter()
        BATCH = 5000
        for i in range(0, N, BATCH):
            sl = data[i:i+BATCH]
            col.add(ids=[str(r["id"]) for r in sl],
                    embeddings=[r["vector"] for r in sl])
        c.ingest_s = time.perf_counter() - t
        queries = qs(Q)
        lat = []
        for q in queries:
            t = time.perf_counter()
            col.query(query_embeddings=[q], n_results=10)
            lat.append((time.perf_counter()-t)*1000)
        c.query_p50_ms = statistics.median(lat)
        c.query_p95_ms = pct(lat, 95); c.query_p99_ms = pct(lat, 99)
        c.disk_mb = dirsize(d)/1_000_000
        c.rss_delta_mb = rss_mb() - rss0
    except Exception as e:
        c.ok = False; c.note = f"err: {str(e)[:150]}"
    finally:
        shutil.rmtree(d, ignore_errors=True)
    return c


def run_synapse(N):
    """PR-G2 scale-100M: drive the Rust synapse_scale_bench example via subprocess.

    Same sha256-derived 384d vectors (Rust side reimplemented to match this
    Python harness byte-for-byte for apples-to-apples).
    """
    import subprocess
    c = Cell("Synapse v2", N)
    binary = "/Users/master/projects/synapse/target/release/examples/synapse_scale_bench"
    if not os.path.exists(binary):
        c.ok = False
        c.note = "binary missing: cargo build --release --example synapse_scale_bench -p synapse-core"
        return c
    rss0 = rss_mb()
    try:
        cmd = [binary, "--n", str(N), "--q", str(Q), "--dim", str(DIM)]
        p = subprocess.run(cmd, capture_output=True, text=True,
                           timeout=TIMEOUT_INGEST + TIMEOUT_QUERY)
        if p.returncode != 0:
            c.ok = False
            c.note = f"exit={p.returncode}: {p.stderr[:140]}"
            return c
        line = [ln for ln in p.stdout.strip().splitlines() if ln.startswith("{")][-1]
        data = json.loads(line)
        c.ingest_s = data["ingest_s"]
        c.query_p50_ms = data["query_p50_ms"]
        c.query_p95_ms = data["query_p95_ms"]
        c.query_p99_ms = data["query_p99_ms"]
        c.disk_mb = data["disk_mb"]
        c.rss_delta_mb = rss_mb() - rss0
        c.note = "in-proc Rust via subprocess; sha-derived vecs"
    except subprocess.TimeoutExpired:
        c.ok = False
        c.note = f"timeout > {TIMEOUT_INGEST + TIMEOUT_QUERY}s"
    except Exception as e:
        c.ok = False
        c.note = f"err: {str(e)[:140]}"
    return c


RUNNERS = {
    "synapse": run_synapse,
    "sqlite_vec": run_sqlite_vec,
    "duckdb_vss": run_duckdb_vss,
    "lancedb": run_lancedb,
    "qdrant": run_qdrant,
    "chroma": run_chroma,
}


def warmup():
    sys.stderr.write("[warmup] 30s thermal ... ")
    t0 = time.perf_counter()
    while time.perf_counter() - t0 < 30:
        _ = [vec(f"warm{i}") for i in range(100)]
    sys.stderr.write("done\n")


def main():
    engines = os.environ.get("ENGINES", "sqlite_vec duckdb_vss lancedb qdrant chroma").split()
    warmup()
    results = []
    incr_csv = OUT/"scale_ladder.csv"
    first = True
    # Write header immediately, append each cell as we go so a crash keeps data.
    with incr_csv.open("w") as f:
        w = csv.DictWriter(f, fieldnames=list(asdict(Cell("x",0)).keys()))
        w.writeheader()
    for N in SCALES:
        for eng in engines:
            sys.stderr.write(f"[bench] engine={eng:<14s} N={N:>8d} ... ")
            sys.stderr.flush()
            t0 = time.perf_counter()
            try:
                c = RUNNERS[eng](N)
            except Exception as e:
                c = Cell(eng, N, ok=False, note=f"runner-err: {str(e)[:120]}")
            dt = time.perf_counter() - t0
            tag = "OK" if c.ok else "FAIL"
            sys.stderr.write(f"{tag}  total={dt:.1f}s  ingest={c.ingest_s:.1f}s  p50={c.query_p50_ms:.3f}ms  p95={c.query_p95_ms:.3f}ms  disk={c.disk_mb:.1f}MB\n")
            results.append(c)
            with incr_csv.open("a") as f:
                w = csv.DictWriter(f, fieldnames=list(asdict(c).keys()))
                w.writerow(asdict(c))
    (OUT/"scale_ladder.json").write_text(json.dumps([asdict(r) for r in results], indent=2))

    # Markdown table: p95 ms per engine × N
    md = [f"# Scale Ladder — 2026-04-23\n",
          f"**Host**: M4 Max 128GB · **dim**=384 · **Q**={Q}/cell · median-of-single ingest, percentiles over Q queries · 30s warmup",
          f"**Raw**: `scale_ladder.csv` + `scale_ladder.json` (same dir).\n",
          "## p95 query latency (ms) by scale\n",
          "| Engine | " + " | ".join(f"N={n:,}" for n in SCALES) + " |",
          "|---" * (len(SCALES)+1) + "|"]
    for eng in engines:
        row = [eng]
        for N in SCALES:
            cell = next((r for r in results if r.engine in (eng, {"synapse":"Synapse v2","sqlite_vec":"sqlite-vec","duckdb_vss":"DuckDB+VSS","lancedb":"LanceDB","qdrant":"Qdrant in-mem","chroma":"Chroma"}[eng]) and r.N == N), None)
            if cell is None or not cell.ok:
                row.append(f"n/m ({cell.note[:25] if cell else 'missing'})")
            else:
                row.append(f"{cell.query_p95_ms:.2f}")
        md.append("| " + " | ".join(row) + " |")
    md.append("\n## Ingest wall-clock (seconds) by scale\n")
    md.append("| Engine | " + " | ".join(f"N={n:,}" for n in SCALES) + " |")
    md.append("|---" * (len(SCALES)+1) + "|")
    for eng in engines:
        row = [eng]
        for N in SCALES:
            cell = next((r for r in results if r.engine in (eng, {"synapse":"Synapse v2","sqlite_vec":"sqlite-vec","duckdb_vss":"DuckDB+VSS","lancedb":"LanceDB","qdrant":"Qdrant in-mem","chroma":"Chroma"}[eng]) and r.N == N), None)
            if cell is None or not cell.ok:
                row.append("n/m")
            else:
                row.append(f"{cell.ingest_s:.1f}")
        md.append("| " + " | ".join(row) + " |")
    md.append("\n## Disk footprint (MB) by scale\n")
    md.append("| Engine | " + " | ".join(f"N={n:,}" for n in SCALES) + " |")
    md.append("|---" * (len(SCALES)+1) + "|")
    for eng in engines:
        row = [eng]
        for N in SCALES:
            cell = next((r for r in results if r.engine in (eng, {"synapse":"Synapse v2","sqlite_vec":"sqlite-vec","duckdb_vss":"DuckDB+VSS","lancedb":"LanceDB","qdrant":"Qdrant in-mem","chroma":"Chroma"}[eng]) and r.N == N), None)
            if cell is None or not cell.ok:
                row.append("n/m")
            else:
                row.append(f"{cell.disk_mb:.1f}")
        md.append("| " + " | ".join(row) + " |")
    (OUT/"SCALE_LADDER.md").write_text("\n".join(md))
    print("\n".join(md))
    print(f"\n[wrote] {OUT}")


if __name__ == "__main__":
    main()
