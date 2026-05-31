#!/usr/bin/env python3
"""CocoIndex-style source delta ledger for Synapse.

This keeps source identity, filters, transform versions, content hashes, and
scan outcomes local and compact. It is the ingestion-side counterpart to the
self-learning recall hook: only changed evidence should become recall material.
"""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import os
import sqlite3
import subprocess
import time
from pathlib import Path


def db_path() -> Path:
    explicit = os.environ.get("SYNAPSE_SOURCE_FLOW_DB")
    if explicit:
        return Path(explicit).expanduser()
    return Path.home() / ".synapse" / "source_flow.db"


def connect() -> sqlite3.Connection:
    path = db_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(path, timeout=1.0)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS source_flows (
            name TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            root TEXT NOT NULL,
            include_globs TEXT NOT NULL,
            exclude_globs TEXT NOT NULL,
            transform_version TEXT NOT NULL,
            created_ts INTEGER NOT NULL,
            updated_ts INTEGER NOT NULL
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS source_items (
            flow TEXT NOT NULL,
            path TEXT NOT NULL,
            uri TEXT,
            content_hash TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            mtime_ns INTEGER NOT NULL,
            status TEXT NOT NULL,
            last_seen_ts INTEGER NOT NULL,
            last_changed_ts INTEGER NOT NULL,
            PRIMARY KEY(flow, path)
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS source_scan_runs (
            id TEXT PRIMARY KEY,
            flow TEXT NOT NULL,
            ts INTEGER NOT NULL,
            seen INTEGER NOT NULL,
            changed INTEGER NOT NULL,
            removed INTEGER NOT NULL,
            duration_ms REAL NOT NULL
        )
        """
    )
    return conn


def stable_hash_bytes(data: bytes) -> str:
    return hashlib.blake2b(data, digest_size=16).hexdigest()


def stable_hash_text(text: str) -> str:
    return stable_hash_bytes(text.encode("utf-8", "ignore"))


def split_globs(items: list[str] | None, default: list[str]) -> list[str]:
    if not items:
        return default
    out: list[str] = []
    for item in items:
        out.extend(part.strip() for part in item.split(",") if part.strip())
    return out or default


def register(args: argparse.Namespace) -> None:
    now = int(time.time())
    include = split_globs(args.include, ["**/*"])
    exclude = split_globs(args.exclude, [".git/**", "target/**", "node_modules/**", "__pycache__/**"])
    root = str(Path(args.root).expanduser().resolve())
    with connect() as conn:
        conn.execute(
            """
            INSERT INTO source_flows(name, kind, root, include_globs, exclude_globs, transform_version, created_ts, updated_ts)
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
            ON CONFLICT(name) DO UPDATE SET
                kind=excluded.kind,
                root=excluded.root,
                include_globs=excluded.include_globs,
                exclude_globs=excluded.exclude_globs,
                transform_version=excluded.transform_version,
                updated_ts=excluded.updated_ts
            """,
            (
                args.name,
                args.kind,
                root,
                json.dumps(include),
                json.dumps(exclude),
                args.transform_version,
                now,
            ),
        )
    print(json.dumps({"registered": args.name, "root": root, "include": include, "exclude": exclude}, indent=2))


def matches(rel: str, includes: list[str], excludes: list[str]) -> bool:
    rel = rel.replace(os.sep, "/")
    included = any(fnmatch.fnmatch(rel, pat) or fnmatch.fnmatch(Path(rel).name, pat) for pat in includes)
    excluded = any(fnmatch.fnmatch(rel, pat) or fnmatch.fnmatch(Path(rel).name, pat) for pat in excludes)
    return included and not excluded


def file_hash(path: Path, max_bytes: int) -> tuple[str, int]:
    size = path.stat().st_size
    h = hashlib.blake2b(digest_size=16)
    with path.open("rb") as fh:
        remaining = max_bytes
        while remaining > 0:
            chunk = fh.read(min(1024 * 1024, remaining))
            if not chunk:
                break
            h.update(chunk)
            remaining -= len(chunk)
    h.update(str(size).encode())
    return h.hexdigest(), size


def load_flow(conn: sqlite3.Connection, name: str) -> dict[str, object]:
    row = conn.execute(
        "SELECT name, kind, root, include_globs, exclude_globs, transform_version FROM source_flows WHERE name=?1",
        (name,),
    ).fetchone()
    if not row:
        raise SystemExit(f"unknown source flow: {name}")
    return {
        "name": row[0],
        "kind": row[1],
        "root": row[2],
        "include": json.loads(row[3]),
        "exclude": json.loads(row[4]),
        "transform_version": row[5],
    }


def scan(args: argparse.Namespace) -> None:
    t0 = time.perf_counter_ns()
    now = int(time.time())
    with connect() as conn:
        flow = load_flow(conn, args.name)
        if flow["kind"] != "file":
            raise SystemExit("only kind=file is implemented in this local ledger slice")
        root = Path(str(flow["root"]))
        includes = list(flow["include"])
        excludes = list(flow["exclude"])
        seen_paths: set[str] = set()
        changed: list[str] = []
        skipped = 0

        for path in root.rglob("*"):
            if not path.is_file():
                continue
            rel = path.relative_to(root).as_posix()
            if not matches(rel, includes, excludes):
                continue
            try:
                stat = path.stat()
                if stat.st_size > args.max_bytes:
                    skipped += 1
                    continue
                content_hash, size = file_hash(path, args.max_bytes)
            except OSError:
                skipped += 1
                continue
            seen_paths.add(rel)
            old = conn.execute(
                "SELECT content_hash FROM source_items WHERE flow=?1 AND path=?2",
                (args.name, rel),
            ).fetchone()
            is_changed = old is None or old[0] != content_hash
            if is_changed:
                changed.append(rel)
            conn.execute(
                """
                INSERT INTO source_items(flow, path, uri, content_hash, size_bytes, mtime_ns, status, last_seen_ts, last_changed_ts)
                VALUES(?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8)
                ON CONFLICT(flow, path) DO UPDATE SET
                    uri=excluded.uri,
                    content_hash=excluded.content_hash,
                    size_bytes=excluded.size_bytes,
                    mtime_ns=excluded.mtime_ns,
                    status='active',
                    last_seen_ts=excluded.last_seen_ts,
                    last_changed_ts=CASE
                        WHEN source_items.content_hash != excluded.content_hash THEN excluded.last_changed_ts
                        ELSE source_items.last_changed_ts
                    END
                """,
                (args.name, rel, path.as_uri(), content_hash, size, stat.st_mtime_ns, now, now if is_changed else now),
            )

        active_rows = conn.execute(
            "SELECT path FROM source_items WHERE flow=?1 AND status='active'",
            (args.name,),
        ).fetchall()
        removed = [row[0] for row in active_rows if row[0] not in seen_paths]
        for rel in removed:
            conn.execute(
                "UPDATE source_items SET status='removed', last_seen_ts=?1 WHERE flow=?2 AND path=?3",
                (now, args.name, rel),
            )

        duration_ms = (time.perf_counter_ns() - t0) / 1_000_000
        run_id = stable_hash_text(f"{args.name}:{now}:{len(seen_paths)}:{len(changed)}:{len(removed)}")
        conn.execute(
            "INSERT OR REPLACE INTO source_scan_runs(id, flow, ts, seen, changed, removed, duration_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (run_id, args.name, now, len(seen_paths), len(changed), len(removed), duration_ms),
        )

    result = {
        "flow": args.name,
        "seen": len(seen_paths),
        "changed": len(changed),
        "removed": len(removed),
        "skipped": skipped,
        "duration_ms": round(duration_ms, 3),
        "changed_paths": changed[: args.preview],
        "removed_paths": removed[: args.preview],
    }
    if args.remember and (changed or removed):
        remember_scan(result)
    print(json.dumps(result, indent=2))


def remember_scan(result: dict[str, object]) -> None:
    synx = os.environ.get("SYNX_BIN") or "synx"
    text = (
        f"source_flow scan {result['flow']}: seen={result['seen']} changed={result['changed']} "
        f"removed={result['removed']} duration_ms={result['duration_ms']}"
    )
    try:
        subprocess.run(
            [synx, "put", "--title", f"source-flow/{result['flow']}", text],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=5,
        )
    except Exception:
        pass


def stats(_args: argparse.Namespace) -> None:
    with connect() as conn:
        flows = conn.execute(
            """
            SELECT f.name, f.kind, f.root, f.transform_version,
                   COUNT(DISTINCT i.path) AS items,
                   COUNT(DISTINCT CASE WHEN i.status='active' THEN i.path END) AS active,
                   MAX(r.ts) AS last_scan_ts
            FROM source_flows f
            LEFT JOIN source_items i ON i.flow=f.name
            LEFT JOIN source_scan_runs r ON r.flow=f.name
            GROUP BY f.name, f.kind, f.root, f.transform_version
            ORDER BY f.name
            """
        ).fetchall()
    print(
        json.dumps(
            [
                {
                    "name": name,
                    "kind": kind,
                    "root": root,
                    "transform_version": transform_version,
                    "items": int(items or 0),
                    "active": int(active or 0),
                    "last_scan_ts": last_scan_ts,
                }
                for name, kind, root, transform_version, items, active, last_scan_ts in flows
            ],
            indent=2,
        )
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("register")
    p.add_argument("--name", required=True)
    p.add_argument("--kind", choices=["file"], default="file")
    p.add_argument("--root", required=True)
    p.add_argument("--include", action="append")
    p.add_argument("--exclude", action="append")
    p.add_argument("--transform-version", default="v1")
    p.set_defaults(func=register)

    p = sub.add_parser("scan")
    p.add_argument("--name", required=True)
    p.add_argument("--max-bytes", type=int, default=5 * 1024 * 1024)
    p.add_argument("--preview", type=int, default=10)
    p.add_argument("--remember", action="store_true")
    p.set_defaults(func=scan)

    p = sub.add_parser("stats")
    p.set_defaults(func=stats)

    args = parser.parse_args()
    args.func(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
