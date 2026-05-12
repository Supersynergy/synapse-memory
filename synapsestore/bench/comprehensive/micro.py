#!/usr/bin/env python3
"""synapse-bench micro — ≤30s iteration benchmark for synapse-turbo development."""
import argparse, json, os, random, sys, tempfile, threading, time
from datetime import datetime
from pathlib import Path

import numpy as np
import pyarrow.parquet as pq

DIR = Path(__file__).parent
RESULTS_DIR = DIR / "results"
LAST_FILE = RESULTS_DIR / ".micro_last.json"
DATASET = DIR / "dataset.parquet"
CATEGORIES = None

N_DOCS = 1000
DAEMON_URL = "http://localhost:9477"

# ─── ANSI colors ───────────────────────────────────────────────────────────────

IS_TTY = sys.stdout.isatty()

def _c(code, text):
    return f"\033[{code}m{text}\033[0m" if IS_TTY else text

green  = lambda t: _c("32", t)
red    = lambda t: _c("31", t)
gray   = lambda t: _c("90", t)
bold   = lambda t: _c("1",  t)
cyan   = lambda t: _c("36", t)

# ─── Dataset ───────────────────────────────────────────────────────────────────

def load_docs(n=N_DOCS):
    global CATEGORIES
    tbl = pq.read_table(DATASET)
    df = tbl.to_pydict()
    total = len(df["id"])
    idx = random.sample(range(total), min(n, total))
    docs = []
    for i in idx:
        vec = df["vec"][i]
        if isinstance(vec, (bytes, bytearray)):
            vec_bytes = bytes(vec)
        else:
            vec_bytes = np.array(vec, dtype=np.float32).tobytes()
        docs.append({
            "id": df["id"][i],
            "text": df["text"][i],
            "category": df["category"][i],
            "score": float(df.get("score", [0.0]*total)[i]),
            "timestamp": int(df.get("timestamp", [0]*total)[i]),
            "source": df.get("source", [""]*(total))[i] if "source" in df else "",
            "lang": df.get("lang", ["en"]*total)[i] if "lang" in df else "en",
            "vec": vec_bytes,
        })
    CATEGORIES = list({d["category"] for d in docs})
    return docs

# ─── Phase A: bulk insert ──────────────────────────────────────────────────────

def phase_a(adapter, docs):
    t0 = time.perf_counter()
    adapter.bulk_insert(docs)
    elapsed = time.perf_counter() - t0
    return len(docs) / elapsed  # ins/s

# ─── Phase D: 8-thread concurrent select 2s ───────────────────────────────────

def _make_thread_adapter(adapter, tmpdir, query_docs=None):
    """Return a thread-safe query function for the adapter."""
    if adapter.name == "sqlite-vec":
        import sqlite3, sqlite_vec as sv
        conn = sqlite3.connect(adapter.db_path, check_same_thread=False)
        conn.enable_load_extension(True)
        sv.load(conn)
        conn.enable_load_extension(False)
        def query(vec, k=10):
            return conn.execute(
                "SELECT id, distance FROM vss WHERE vec MATCH ? AND k=?", (vec, k)
            ).fetchall()
        return query
    elif adapter.name == "synapse":
        # Daemon uses text queries: /vec?q=<text>&k=N
        sample_texts = [d["text"][:60] for d in (query_docs if query_docs else [])]
        import urllib.request, urllib.parse
        def query(vec, k=10):
            q = random.choice(sample_texts) if sample_texts else "test"
            url = f"{DAEMON_URL}/vec?q={urllib.parse.quote(q)}&k={k}"
            with urllib.request.urlopen(url, timeout=5) as r:
                return json.loads(r.read()).get("results", [])
        return query
    else:
        return lambda vec, k=10: adapter.vec_only_select(vec, k)


def phase_d(adapter, query_docs, tmpdir):
    ops = 0
    lock = threading.Lock()
    stop = threading.Event()
    query_fn = _make_thread_adapter(adapter, tmpdir, query_docs)

    def worker():
        nonlocal ops
        while not stop.is_set():
            d = random.choice(query_docs)
            try:
                query_fn(d["vec"], k=10)
                with lock:
                    ops += 1
            except Exception:
                pass

    threads = [threading.Thread(target=worker, daemon=True) for _ in range(8)]
    for t in threads: t.start()
    time.sleep(2.0)
    stop.set()
    for t in threads: t.join(timeout=1)
    return ops / 2.0  # ops/s

# ─── Phase E: recall@10 with 20 queries ───────────────────────────────────────

def _extract_ids(results):
    ids = set()
    for r in results:
        if isinstance(r, (list, tuple)):
            ids.add(str(r[0]))
        elif isinstance(r, dict):
            ids.add(str(r.get("id", r.get("doc_id", ""))))
        else:
            ids.add(str(r))
    return ids

def phase_e(adapter, ground_truth_adapter, query_docs):
    queries = random.sample(query_docs, min(20, len(query_docs)))
    hits = 0
    for d in queries:
        gt = ground_truth_adapter.vec_only_select(d["vec"], k=10)
        got = adapter.vec_only_select(d["vec"], k=10)
        gt_ids  = _extract_ids(gt)
        got_ids = _extract_ids(got)
        hits += len(gt_ids & got_ids) / max(len(gt_ids), 1) if gt_ids else 0
    return hits / len(queries)

# ─── Phase I: cache hitrate 200 repeats ───────────────────────────────────────

def phase_i(adapter, query_docs):
    sample = random.sample(query_docs, min(5, len(query_docs)))
    # cold pass
    t0 = time.perf_counter()
    for d in sample:
        adapter.vec_only_select(d["vec"], k=10)
    cold = (time.perf_counter() - t0) / len(sample)

    # warm pass (200 repeats of same queries)
    t0 = time.perf_counter()
    repeats = 200
    for _ in range(repeats):
        d = random.choice(sample)
        adapter.vec_only_select(d["vec"], k=10)
    warm = (time.perf_counter() - t0) / repeats

    speedup = cold / warm if warm > 0 else 1.0
    return speedup

# ─── Daemon health check ───────────────────────────────────────────────────────

def check_daemon():
    import urllib.request, urllib.error
    try:
        with urllib.request.urlopen(f"{DAEMON_URL}/health", timeout=2) as r:
            info = json.loads(r.read())
        uptime_h = info.get("uptime_s", 0) / 3600
        hitrate = info.get("hitrate", info.get("cache_hitrate", 0)) * 100
        docs = info.get("numpy_docs", info.get("docs", "?"))
        print(cyan(f"daemon: ok") + gray(f" (uptime {uptime_h:.1f}h, hitrate {hitrate:.1f}%, {docs} docs)"))
        return True
    except Exception as e:
        print(red(f"daemon: DOWN ({e})"))
        return False

# ─── Engine registry ───────────────────────────────────────────────────────────

def get_adapter(name):
    sys.path.insert(0, str(DIR))
    from bench import SqliteVecAdapter, SynapseAdapter, DuckDBAdapter
    mapping = {
        "sqlite-vec": SqliteVecAdapter,
        "synapse":    SynapseAdapter,
        "duckdb":     DuckDBAdapter,
    }
    cls = mapping.get(name)
    if cls is None:
        print(red(f"Unknown engine: {name}. Choose from: {', '.join(mapping)}"))
        sys.exit(1)
    return cls()

# ─── Delta display ─────────────────────────────────────────────────────────────

def show_delta(current, last, no_delta):
    if no_delta or last is None:
        return
    print()
    print(bold(f"delta vs last run ({LAST_FILE.name}):"))
    for engine, metrics in current.items():
        for key, val in metrics.items():
            last_val = last.get(engine, {}).get(key)
            if last_val is None or last_val == 0:
                continue
            if val != val or last_val != last_val:  # NaN check
                continue
            pct = (val - last_val) / last_val * 100
            label = f"  {engine} {key}:"
            if pct > 5:
                print(f"{label}   {green(f'+{pct:.1f}%')}")
            elif pct < -5:
                print(f"{label}   {red(f'{pct:.1f}%')}")
            else:
                print(f"{label}   {gray(f'{pct:+.1f}%')}")

# ─── Main ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engines", default="sqlite-vec,synapse",
                        help="Comma-separated engine list (default: sqlite-vec,synapse)")
    parser.add_argument("--no-delta", action="store_true", help="Skip delta comparison")
    args = parser.parse_args()

    engine_names = [e.strip() for e in args.engines.split(",")]

    ts = datetime.now().strftime("%Y-%m-%d %H:%M")
    print(bold(f"synapse-bench micro · {N_DOCS} docs · {ts}"))

    daemon_ok = check_daemon()
    if "synapse" in engine_names and not daemon_ok:
        print(red("ERROR: synapse engine requires daemon. Start with: launchctl start de.supersynergy.synapse-turbo"))
        sys.exit(1)

    print()
    docs = load_docs(N_DOCS)

    last_results = None
    if LAST_FILE.exists() and not args.no_delta:
        try:
            last_results = json.loads(LAST_FILE.read_text())
        except Exception:
            pass

    current_results = {}
    rows = []

    with tempfile.TemporaryDirectory() as tmpdir:
        # sqlite-vec is ground-truth for recall
        ground_truth = None
        gt_tmpdir = tmpdir

        for name in engine_names:
            print(gray(f"  [{name}] setup..."), end=" ", flush=True)
            adapter = get_adapter(name)
            try:
                adapter.setup(tmpdir)
            except Exception as e:
                print(red(f"SKIP ({e})"))
                continue

            # Phase A
            print(gray("A..."), end=" ", flush=True)
            ins_s = phase_a(adapter, docs)

            # Build ground truth from first engine
            if ground_truth is None:
                ground_truth = adapter

            # Phase D
            print(gray("D..."), end=" ", flush=True)
            ops_s = phase_d(adapter, docs, tmpdir)

            # Phase E — recall vs ground-truth (sqlite-vec); N/A for daemon-backed engines
            print(gray("E..."), end=" ", flush=True)
            if adapter.name == "synapse" and adapter._use_daemon:
                recall = float("nan")
            else:
                recall = phase_e(adapter, ground_truth, docs)

            # Phase I
            print(gray("I..."), end=" ", flush=True)
            speedup = phase_i(adapter, docs)

            print(gray("done"))

            current_results[name] = {
                "A": ins_s, "D": ops_s, "E": recall, "I": speedup
            }
            rows.append((name, ins_s, ops_s, recall, speedup))

            if adapter is not ground_truth:
                adapter.teardown()

        if ground_truth:
            ground_truth.teardown()

    # Table
    print()
    hdr = f"{'engine':<14}  {'A.ins/s':>10}  {'D.ops/s':>10}  {'E.recall':>9}  {'I.speedup':>10}"
    sep = "─" * len(hdr)
    print(bold(hdr))
    print(gray(sep))
    for name, ins_s, ops_s, recall, speedup in rows:
        rec_str = "       N/A" if (recall != recall) else f"{recall:>9.3f}"  # NaN check
        print(f"{name:<14}  {ins_s:>10,.0f}  {ops_s:>10,.0f}  {rec_str}  {speedup:>9.1f}×")

    show_delta(current_results, last_results, args.no_delta)

    # Save snapshot
    RESULTS_DIR.mkdir(exist_ok=True)
    LAST_FILE.write_text(json.dumps(current_results, indent=2))
    print(gray(f"\nsnapshot → {LAST_FILE}"))


if __name__ == "__main__":
    main()
