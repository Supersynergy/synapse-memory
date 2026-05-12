#!/usr/bin/env python3
"""
Synapse config sweep harness.
Parametrizes SQLite + synapse-engine knobs, runs insert+query on lme_s_50,
records latency/throughput, writes results.jsonl.

Usage:
    python harness.py [--configs N]   # N random configs (default 30)
    python harness.py --full           # exhaustive 432-point grid
"""
from __future__ import annotations

import argparse
import itertools
import json
import os
import pathlib
import random
import sqlite3
import tempfile
import time

LME_DATA = pathlib.Path(__file__).parent.parent / "longmemeval" / "data" / "lme_s_50.json"
RESULTS_FILE = pathlib.Path(__file__).parent / "results.jsonl"

GRID = {
    "cache_size_mb": [64, 256, 1024],
    "mmap_size_mb": [0, 256, 1024],
    "page_size": [4096, 8192, 16384],
    "journal_mode": ["WAL", "MEMORY"],
    "synchronous": ["NORMAL", "OFF"],
    "batch_size": [1, 100, 1000, 10000],
}


def apply_config(conn: sqlite3.Connection, cfg: dict) -> None:
    cache_pages = (cfg["cache_size_mb"] * 1024 * 1024) // cfg["page_size"]
    conn.execute(f"PRAGMA cache_size = -{cache_pages}")
    mmap_bytes = cfg["mmap_size_mb"] * 1024 * 1024
    conn.execute(f"PRAGMA mmap_size = {mmap_bytes}")
    conn.execute(f"PRAGMA page_size = {cfg['page_size']}")
    conn.execute(f"PRAGMA journal_mode = {cfg['journal_mode']}")
    conn.execute(f"PRAGMA synchronous = {cfg['synchronous']}")


def run_config(cfg: dict, docs: list[dict]) -> dict:
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "bench.db")
        conn = sqlite3.connect(db_path)

        # Apply pragmas before schema creation
        apply_config(conn, cfg)

        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(id, body, tokenize='porter ascii')"
        )
        conn.execute(
            "CREATE TABLE IF NOT EXISTS chunks (id TEXT PRIMARY KEY, body TEXT, ts REAL)"
        )
        conn.commit()

        batch = cfg["batch_size"]
        rows = [(d.get("qid", str(i)), d.get("question", "")[:2000]) for i, d in enumerate(docs)]

        # Insert benchmark
        t0 = time.perf_counter()
        for start in range(0, len(rows), batch):
            chunk = rows[start : start + batch]
            conn.executemany("INSERT OR IGNORE INTO chunks VALUES (?,?,?)", [(r[0], r[1], time.time()) for r in chunk])
            conn.executemany("INSERT OR IGNORE INTO fts VALUES (?,?)", chunk)
            conn.commit()
        insert_s = time.perf_counter() - t0
        insert_ops_s = len(rows) / insert_s if insert_s > 0 else 0

        # Query benchmark — FTS5 search
        raw_queries = [d.get("question", "memory") for d in docs[:20]]
        # FTS5 MATCH needs simple single-word tokens; extract first word
        queries = []
        for q in raw_queries:
            word = q.strip().split()[0] if q.strip() else "memory"
            word = "".join(c for c in word if c.isalnum())
            queries.append(word or "memory")
        latencies = []
        for q in queries:
            t0 = time.perf_counter()
            conn.execute("SELECT id FROM fts WHERE fts MATCH ? LIMIT 5", (q,)).fetchall()
            latencies.append((time.perf_counter() - t0) * 1000)

        latencies.sort()
        p50 = latencies[len(latencies) // 2] if latencies else 0
        p99 = latencies[int(len(latencies) * 0.99)] if latencies else 0

        db_size_kb = os.path.getsize(db_path) / 1024
        conn.close()

    return {
        **cfg,
        "insert_ops_s": round(insert_ops_s, 1),
        "query_p50_ms": round(p50, 4),
        "query_p99_ms": round(p99, 4),
        "db_size_kb": round(db_size_kb, 1),
        "n_docs": len(rows),
    }


def all_configs() -> list[dict]:
    keys = list(GRID.keys())
    return [dict(zip(keys, vals)) for vals in itertools.product(*GRID.values())]


def sample_configs(n: int) -> list[dict]:
    keys = list(GRID.keys())
    seen = set()
    result = []
    attempts = 0
    while len(result) < n and attempts < n * 10:
        attempts += 1
        cfg = {k: random.choice(v) for k, v in GRID.items()}
        key = tuple(cfg[k] for k in keys)
        if key not in seen:
            seen.add(key)
            result.append(cfg)
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--configs", type=int, default=30)
    parser.add_argument("--full", action="store_true")
    args = parser.parse_args()

    if not LME_DATA.exists():
        print(f"ERROR: dataset not found at {LME_DATA}")
        raise SystemExit(1)

    docs = json.loads(LME_DATA.read_text())
    print(f"Loaded {len(docs)} docs from {LME_DATA}")

    configs = all_configs() if args.full else sample_configs(args.configs)
    print(f"Running {len(configs)} configs...")

    RESULTS_FILE.parent.mkdir(parents=True, exist_ok=True)
    RESULTS_FILE.unlink(missing_ok=True)

    best: dict | None = None
    for i, cfg in enumerate(configs, 1):
        result = run_config(cfg, docs)
        with RESULTS_FILE.open("a") as f:
            f.write(json.dumps(result) + "\n")
        score = result["insert_ops_s"] / max(result["query_p50_ms"], 0.001)
        if best is None or score > best["_score"]:
            result["_score"] = score
            best = result
        print(
            f"[{i:3d}/{len(configs)}] "
            f"cache={cfg['cache_size_mb']}MB mmap={cfg['mmap_size_mb']}MB "
            f"page={cfg['page_size']} jm={cfg['journal_mode']} "
            f"sync={cfg['synchronous']} batch={cfg['batch_size']:5d} → "
            f"ins={result['insert_ops_s']:8.0f} ops/s  q_p50={result['query_p50_ms']:.3f}ms"
        )

    print(f"\nBest config (highest insert_ops_s/query_p50 ratio):")
    if best:
        for k, v in best.items():
            if not k.startswith("_"):
                print(f"  {k}: {v}")
    print(f"\nFull results: {RESULTS_FILE}")


if __name__ == "__main__":
    main()
