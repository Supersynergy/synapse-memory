#!/usr/bin/env python3
"""bench_history.py — record bench results into ~/.synapse-x/bench-history.db

Usage:
  bench_history.py record --bench <name> --p50 <µs> --mean <µs> [--commit <sha>] [--machine <tag>]
  bench_history.py ingest <bench_txt_file> [--bench <name>] [--commit <sha>] [--machine <tag>]
  bench_history.py --report [--days 30]
"""
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///

import argparse
import os
import re
import sqlite3
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

DB_PATH = Path.home() / ".synapse-x" / "bench-history.db"


def get_db() -> sqlite3.Connection:
    DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(DB_PATH)
    conn.execute("""
        CREATE TABLE IF NOT EXISTS bench_runs (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            ts          TEXT    NOT NULL,
            commit_sha  TEXT    NOT NULL,
            bench_name  TEXT    NOT NULL,
            p50_us      REAL,
            mean_us     REAL,
            machine_tag TEXT    NOT NULL DEFAULT 'local'
        )
    """)
    conn.execute("CREATE INDEX IF NOT EXISTS idx_bench_ts ON bench_runs(bench_name, ts)")
    conn.commit()
    return conn


def git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"], stderr=subprocess.DEVNULL
        ).decode().strip()
    except Exception:
        return "unknown"


def insert_run(conn, bench_name, p50_us, mean_us, commit_sha, machine_tag):
    ts = datetime.now(timezone.utc).isoformat()
    conn.execute(
        "INSERT INTO bench_runs (ts, commit_sha, bench_name, p50_us, mean_us, machine_tag) "
        "VALUES (?, ?, ?, ?, ?, ?)",
        (ts, commit_sha, bench_name, p50_us, mean_us, machine_tag),
    )
    conn.commit()
    print(f"Recorded: {bench_name}  p50={p50_us}µs  mean={mean_us}µs  sha={commit_sha}")


def parse_criterion_txt(txt: str, bench_name_hint: str | None = None):
    """Extract (bench_name, p50_us, mean_us) tuples from criterion stdout."""
    results = []
    # Criterion output:  bench_name  time:   [low µs  MEDIAN µs  high µs]
    pattern = re.compile(
        r"^(\S[\w/: ]+?)\s+time:\s+\["
        r"([\d.]+)\s+(µs|ns|ms)\s+([\d.]+)\s+(µs|ns|ms)\s+([\d.]+)\s+(µs|ns|ms)\]",
        re.MULTILINE,
    )
    for m in pattern.finditer(txt):
        name = m.group(1).strip()
        # low=m.group(2), median=m.group(4), high=m.group(6)
        median_val, median_unit = float(m.group(4)), m.group(5)
        mean_val, mean_unit = float(m.group(6)), m.group(7)
        def to_us(v, u):
            if u == 'ns': return v / 1000
            if u == 'ms': return v * 1000
            return v
        p50_us = to_us(median_val, median_unit)
        mean_us = to_us(mean_val, mean_unit)
        results.append((name, p50_us, mean_us))

    # Throughput lines: thrpt: [... MEDIAN Melem/s ...]
    tp_pattern = re.compile(
        r"^(\S[\w/: ]+?)\s+thrpt:\s+\["
        r"[\d.]+ \S+\s+([\d.]+)\s+(Melem/s|Kelem/s|elem/s)",
        re.MULTILINE,
    )
    for m in tp_pattern.finditer(txt):
        name = m.group(1).strip()
        val, unit = float(m.group(2)), m.group(3)
        if unit == 'Melem/s': eps = val * 1_000_000
        elif unit == 'Kelem/s': eps = val * 1_000
        else: eps = val
        # store eps as negative µs sentinel so schema stays numeric (1/eps in µs)
        p50_us = 1_000_000 / eps if eps > 0 else None
        results.append((name + "_thrpt", p50_us, p50_us))

    if bench_name_hint and results:
        results = [(bench_name_hint if i == 0 else f"{bench_name_hint}/{n}", p, me)
                   for i, (n, p, me) in enumerate(results)]
    return results


def cmd_record(args):
    conn = get_db()
    sha = args.commit or git_sha()
    machine = args.machine or os.uname().machine
    insert_run(conn, args.bench, args.p50, args.mean, sha, machine)


def cmd_ingest(args):
    conn = get_db()
    sha = args.commit or git_sha()
    machine = args.machine or os.uname().machine
    txt = Path(args.file).read_text()
    results = parse_criterion_txt(txt, bench_name_hint=args.bench)
    if not results:
        print(f"No criterion results found in {args.file}", file=sys.stderr)
        sys.exit(1)
    for (name, p50, mean) in results:
        insert_run(conn, name, p50, mean, sha, machine)


def cmd_report(args):
    conn = get_db()
    days = args.days
    rows = conn.execute("""
        SELECT bench_name,
               strftime('%Y-%m-%d', ts) as day,
               AVG(p50_us) as avg_p50,
               MIN(p50_us) as min_p50,
               MAX(p50_us) as max_p50,
               COUNT(*) as n
        FROM bench_runs
        WHERE ts >= datetime('now', ?)
        GROUP BY bench_name, day
        ORDER BY bench_name, day
    """, (f"-{days} days",)).fetchall()

    if not rows:
        print(f"No data in last {days} days.")
        return

    # Group by bench_name
    from collections import defaultdict
    by_bench = defaultdict(list)
    for (bn, day, avg_p50, min_p50, max_p50, n) in rows:
        by_bench[bn].append((day, avg_p50, min_p50, max_p50, n))

    print(f"# Bench Trend Report — last {days} days\n")
    for bench_name, entries in sorted(by_bench.items()):
        print(f"## {bench_name}\n")
        print("| date | avg p50 µs | min µs | max µs | runs |")
        print("|------|------------|--------|--------|------|")
        for (day, avg_p50, min_p50, max_p50, n) in entries:
            print(f"| {day} | {avg_p50:.2f} | {min_p50:.2f} | {max_p50:.2f} | {n} |")
        print()


def main():
    parser = argparse.ArgumentParser(description="bench_history — record and report bench results")
    sub = parser.add_subparsers(dest="cmd")

    p_record = sub.add_parser("record", help="Record a single bench result")
    p_record.add_argument("--bench", required=True)
    p_record.add_argument("--p50", type=float, required=True)
    p_record.add_argument("--mean", type=float, required=True)
    p_record.add_argument("--commit")
    p_record.add_argument("--machine")

    p_ingest = sub.add_parser("ingest", help="Parse criterion txt file and record all results")
    p_ingest.add_argument("file")
    p_ingest.add_argument("--bench", help="Override bench name prefix")
    p_ingest.add_argument("--commit")
    p_ingest.add_argument("--machine")

    p_report = sub.add_parser("report", help="Print markdown trend table")
    p_report.add_argument("--days", type=int, default=30)

    # legacy --report flag at top level
    parser.add_argument("--report", action="store_true", help="Print markdown trend table")
    parser.add_argument("--days", type=int, default=30)

    args = parser.parse_args()

    if args.report or args.cmd is None and hasattr(args, 'report') and args.report:
        cmd_report(args)
    elif args.cmd == "record":
        cmd_record(args)
    elif args.cmd == "ingest":
        cmd_ingest(args)
    elif args.cmd == "report":
        cmd_report(args)
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    main()
