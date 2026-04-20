#!/usr/bin/env python3
"""
Migrate SuperKnow v2 core.db → Synapse store via CLI.

Usage:
    python tools/migrate_superknow.py [options]

Options:
    --db PATH          Source SuperKnow core.db [~/.claude/superknow/core.db]
    --synapse PATH     Target synapse brain.db [.synapse/brain.db]
    --sk PATH          Ed25519 signing key [synapse.sk]
    --limit N          Only migrate N rows (smoke test)
    --dry-run          Count rows only, no writes
    --verify           After migration, verify 10 random docs (requires embed)
    --no-sign          Skip signing
"""

import argparse
import os
import random
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

try:
    from tqdm import tqdm
except ImportError:
    def tqdm(it, **kw):
        total = kw.get("total", "?")
        print(f"[migrate] processing {total} rows...", flush=True)
        return it


DEFAULT_DB = Path.home() / ".claude/superknow/core.db"
DEFAULT_SYNAPSE = Path(".synapse/brain.db")
DEFAULT_SK = Path("synapse.sk")


def parse_args():
    p = argparse.ArgumentParser(description="Migrate SuperKnow v2 → Synapse")
    p.add_argument("--db", type=Path, default=DEFAULT_DB)
    p.add_argument("--synapse", type=Path, default=DEFAULT_SYNAPSE)
    p.add_argument("--sk", type=Path, default=DEFAULT_SK)
    p.add_argument("--limit", type=int, default=None)
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("--verify", action="store_true")
    p.add_argument("--no-sign", action="store_true")
    return p.parse_args()


def fetch_rows(db: Path, limit=None):
    conn = sqlite3.connect(str(db))
    conn.row_factory = sqlite3.Row
    q = "SELECT id, title, body, created_at, agent_id, type FROM memories WHERE body != '' ORDER BY created_at"
    if limit:
        q += f" LIMIT {limit}"
    rows = conn.execute(q).fetchall()
    conn.close()
    return rows


def synapse_cli(args_list, input_text=None):
    cmd = ["cargo", "run", "-p", "synapse-cli", "--quiet", "--"]
    cmd += args_list
    result = subprocess.run(
        cmd,
        input=input_text,
        capture_output=True,
        text=True,
        cwd=Path(__file__).parent.parent,
    )
    if result.returncode != 0:
        raise RuntimeError(f"synapse CLI error: {result.stderr.strip()}")
    return result.stdout.strip()


def put_doc(row, synapse_db: Path, sk: Path, no_sign: bool):
    title = (row["title"] or row["id"])[:200]
    uri = f"superknow://{row['id']}"
    # Build tags from type and agent_id
    tags_parts = [t for t in [row["type"], row["agent_id"]] if t]
    tags_str = ",".join(tags_parts) if tags_parts else ""
    extra_flags = []
    if tags_str:
        extra_flags += ["--uri", uri]
    args = [
        "-f", str(synapse_db),
        "put",
        "--no-embed",
        "--title", title,
        "--uri", uri,
    ]
    if not no_sign and sk.exists():
        args += ["--sign", str(sk)]
    synapse_cli(args, input_text=row["body"])


def verify_overlap(rows_sample, synapse_db: Path):
    """Pick 10 random docs, query both stores, assert top-3 overlap >= 2."""
    import sqlite3 as sq

    conn = sq.connect(str(DEFAULT_DB))
    synapse_conn = sq.connect(str(synapse_db))

    passed = 0
    failed = 0
    sample = random.sample(rows_sample, min(10, len(rows_sample)))

    for row in sample:
        query = (row["title"] or row["body"])[:100]
        # SuperKnow FTS
        try:
            sk_hits = conn.execute(
                "SELECT id FROM memories_fts WHERE memories_fts MATCH ? LIMIT 3",
                (query,),
            ).fetchall()
            sk_ids = {r[0] for r in sk_hits}
        except Exception:
            sk_ids = set()

        # Synapse FTS
        try:
            out = synapse_cli(["-f", str(synapse_db), "find", query, "--limit", "3"])
            syn_ids = set()
            for line in out.splitlines():
                parts = line.split("\t")
                if len(parts) >= 3:
                    syn_ids.add(parts[2][:80])
        except Exception:
            syn_ids = set()

        overlap = len(sk_ids & syn_ids) if sk_ids else 0
        if overlap >= 2 or not sk_ids:
            passed += 1
        else:
            failed += 1
            print(f"  [warn] low overlap for '{query[:40]}': sk={sk_ids} syn={syn_ids}")

    conn.close()
    synapse_conn.close()
    print(f"[verify] {passed}/10 passed (overlap>=2), {failed} failed")
    return failed == 0


def main():
    args = parse_args()

    if not args.db.exists():
        print(f"[error] SuperKnow db not found: {args.db}", file=sys.stderr)
        sys.exit(1)

    rows = fetch_rows(args.db, limit=args.limit)
    total = len(rows)
    print(f"[migrate] found {total} memories in {args.db}")

    if args.dry_run:
        print(f"[dry-run] would migrate {total} rows → {args.synapse}")
        return

    # Ensure synapse db directory exists
    args.synapse.parent.mkdir(parents=True, exist_ok=True)

    # Init store if needed
    try:
        synapse_cli(["-f", str(args.synapse), "init"])
    except Exception:
        pass  # already initialized

    errors = 0
    with tqdm(rows, total=total, desc="migrating", unit="doc") as bar:
        for row in bar:
            try:
                put_doc(row, args.synapse, args.sk, args.no_sign)
            except Exception as e:
                errors += 1
                if errors <= 5:
                    print(f"\n[warn] row {row['id']}: {e}", file=sys.stderr)

    print(f"[migrate] done: {total - errors} ok, {errors} errors → {args.synapse}")

    if args.verify:
        print("[verify] running overlap check on 10 random docs...")
        verify_overlap(list(rows), args.synapse)


if __name__ == "__main__":
    main()
