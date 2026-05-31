#!/usr/bin/env python3
"""Backfill embeddings for docs with no docs_vec row.

Finds docs.id NOT IN docs_vec, embeds via the existing MLX BGE sidecar
(synapse-mlx-embed.py, CLS-pool + L2-norm, 384-dim), and inserts the
vectors into docs_vec.

Safe to run while synapsed is alive (brain.db is WAL — long busy_timeout
+ batched IMMEDIATE transactions).
"""
from __future__ import annotations

import argparse
import os
import struct
import subprocess
import sys
import time

import msgpack
import numpy as np
import sqlite3
import sqlite_vec

DB = "/Users/master/.synapse/brain.db"
SIDECAR = "/Users/master/projects/synapse/scripts/synapse-mlx-embed.py"
PYTHON = "/Users/master/.venvs/synapse-backfill-314/bin/python"
DIM = 384


def _read_exact(fh, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = fh.read(n - len(buf))
        if not chunk:
            return None
        buf.extend(chunk)
    return bytes(buf)


def _read_msg(fh):
    hdr = _read_exact(fh, 4)
    if hdr is None:
        return None
    (n,) = struct.unpack(">I", hdr)
    body = _read_exact(fh, n)
    if body is None:
        return None
    return msgpack.unpackb(body, raw=False)


def _write_msg(fh, obj):
    body = msgpack.packb(obj, use_bin_type=True)
    fh.write(struct.pack(">I", len(body)))
    fh.write(body)
    fh.flush()


def open_db():
    db = sqlite3.connect(DB, timeout=60.0, isolation_level=None)
    db.enable_load_extension(True)
    sqlite_vec.load(db)
    db.execute("PRAGMA busy_timeout=60000")
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("PRAGMA synchronous=NORMAL")
    return db


def spawn_embedder():
    env = os.environ.copy()
    env.setdefault("SYNAPSE_MLX_MODEL_PATH", "/Users/master/projects/synapse/models/bge-small-mlx-bf16")
    return subprocess.Popen(
        [PYTHON, SIDECAR],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        env=env,
        bufsize=0,
    )


def embed_batch(proc, texts):
    _write_msg(proc.stdin, {"texts": texts})
    resp = _read_msg(proc.stdout)
    if resp is None:
        raise RuntimeError("embedder closed")
    if "error" in resp:
        raise RuntimeError(f"embedder error: {resp['error']}")
    return resp["vecs"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--batch", type=int, default=64)
    ap.add_argument("--commit-every", type=int, default=512)
    ap.add_argument("--limit", type=int, default=0, help="0 = all")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    db = open_db()
    total = db.execute(
        "SELECT COUNT(*) FROM docs d WHERE NOT EXISTS (SELECT 1 FROM docs_vec v WHERE v.id=d.id)"
    ).fetchone()[0]
    if args.limit:
        total = min(total, args.limit)
    print(f"orphans to backfill: {total}", flush=True)
    if args.dry_run or total == 0:
        return

    proc = spawn_embedder()
    print("embedder spawned, loading model...", flush=True)
    handshake = _read_msg(proc.stdout)
    print(f"ready: {handshake}", flush=True)
    embed_batch(proc, ["warm-up"])
    print("model warm.", flush=True)

    cur = db.cursor()
    done = 0
    pending = 0
    t0 = time.monotonic()
    cur.execute("BEGIN IMMEDIATE")

    sql_orphans = (
        "SELECT d.id, d.text FROM docs d "
        "WHERE NOT EXISTS (SELECT 1 FROM docs_vec v WHERE v.id=d.id) "
        "ORDER BY d.id"
    )
    if args.limit:
        sql_orphans += f" LIMIT {args.limit}"

    read = open_db()
    batch_ids: list[int] = []
    batch_text: list[str] = []
    for row_id, text in read.execute(sql_orphans):
        batch_ids.append(int(row_id))
        batch_text.append(text or "")
        if len(batch_ids) < args.batch:
            continue
        vecs = embed_batch(proc, batch_text)
        for rid, v in zip(batch_ids, vecs):
            arr = np.asarray(v, dtype=np.float32)
            if arr.shape[0] != DIM:
                raise RuntimeError(f"dim mismatch {arr.shape[0]}")
            cur.execute("INSERT INTO docs_vec(id, embedding) VALUES (?, ?)", (rid, arr.tobytes()))
        done += len(batch_ids)
        pending += len(batch_ids)
        batch_ids.clear()
        batch_text.clear()
        if pending >= args.commit_every:
            cur.execute("COMMIT")
            pending = 0
            cur.execute("BEGIN IMMEDIATE")
            dt = time.monotonic() - t0
            eta = (total - done) * dt / max(done, 1)
            print(f"  {done}/{total}  ({done/dt:.1f} doc/s, ETA {eta/60:.1f}min)", flush=True)

    if batch_ids:
        vecs = embed_batch(proc, batch_text)
        for rid, v in zip(batch_ids, vecs):
            arr = np.asarray(v, dtype=np.float32)
            cur.execute("INSERT INTO docs_vec(id, embedding) VALUES (?, ?)", (rid, arr.tobytes()))
        done += len(batch_ids)
    cur.execute("COMMIT")
    dt = time.monotonic() - t0
    print(f"DONE: {done} docs embedded in {dt/60:.1f}min  ({done/dt:.1f} doc/s)")
    proc.stdin.close()
    proc.wait(timeout=5)


if __name__ == "__main__":
    main()
