#!/usr/bin/env python3
"""trend.py — push bench results into Synapse brain.db and track regressions.

Usage:
  python3 trend.py                    # seed brain.db with current results
  python3 trend.py --compare last:5  # show delta table for last 5 runs
  python3 trend.py --regression-check # exit 1 if >10% regression vs prev
  python3 trend.py --help
"""

import argparse
import json
import os
import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path

DIR = Path(__file__).parent
RESULTS_DIR = DIR / "results"
SYNX_BIN = Path.home() / ".local/bin/synx"
SYNAPSE_BIN = Path.home() / "projects/synapse/target/release/synapse"

# ── synx CLI helpers ───────────────────────────────────────────────────────────

def synx_cmd() -> str:
    if SYNX_BIN.exists():
        return str(SYNX_BIN)
    if SYNAPSE_BIN.exists():
        return str(SYNAPSE_BIN)
    raise FileNotFoundError("synx/synapse binary not found. Checked: ~/.local/bin/synx and ~/projects/synapse/target/release/synapse")


def synx_put(title: str, body: str) -> bool:
    """Push a document to brain.db via `synx put`."""
    try:
        bin_ = synx_cmd()
        result = subprocess.run(
            [bin_, "put", "--title", title],
            input=body, text=True, capture_output=True, timeout=10
        )
        if result.returncode != 0:
            print(f"[trend] synx put failed: {result.stderr.strip()}", file=sys.stderr)
            return False
        return True
    except Exception as e:
        print(f"[trend] synx put error: {e}", file=sys.stderr)
        return False


def synx_search(query: str, limit: int = 20) -> list[dict]:
    """Search brain.db via `synx hybrid`."""
    try:
        bin_ = synx_cmd()
        result = subprocess.run(
            [bin_, "hybrid", query, str(limit)],
            capture_output=True, text=True, timeout=10
        )
        if result.returncode != 0:
            return []
        docs = []
        for line in result.stdout.splitlines():
            line = line.strip()
            if not line:
                continue
            # Try JSON parse first
            try:
                docs.append(json.loads(line))
                continue
            except json.JSONDecodeError:
                pass
            # Fallback: parse "title: ... body: ..." format
            docs.append({"raw": line})
        return docs
    except Exception as e:
        print(f"[trend] synx search error: {e}", file=sys.stderr)
        return []


# ── Result loading ────────────────────────────────────────────────────────────

def load_results() -> list[dict]:
    rows = []
    for f in sorted(RESULTS_DIR.glob("*.jsonl")):
        if f.name.startswith("summary"):
            continue
        engine = f.stem.rsplit("_", 1)[0]
        profile = f.stem.rsplit("_", 1)[1]
        with open(f) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    d = json.loads(line)
                    d["_engine"] = engine
                    d["_profile"] = profile
                    rows.append(d)
                except json.JSONDecodeError:
                    pass
    return rows


def extract_summary(row: dict) -> dict:
    """Extract key metrics from a result row."""
    eng = row.get("engine", row.get("_engine", "unknown"))
    profile = row.get("profile", row.get("_profile", "unknown"))
    summary = {
        "engine": eng,
        "profile": profile,
        "phase_a_ops_sec": row.get("phase_a", {}).get("ops_sec", 0),
        "phase_b_ops_sec": row.get("phase_b", {}).get("ops_sec", 0),
        "phase_c_ops_sec": row.get("phase_c", {}).get("ops_sec", 0),
        "phase_d_ops_sec": row.get("phase_d", {}).get("ops_sec", 0),
        "recall_at_10": row.get("phase_e", {}).get("recall_at_10", None),
        "p50_ms": row.get("phase_c", {}).get("p50_ms", None) or row.get("phase_d", {}).get("p50_ms", None),
        "p95_ms": row.get("phase_c", {}).get("p95_ms", None) or row.get("phase_d", {}).get("p95_ms", None),
        "rss_mb": max(
            row.get("phase_a", {}).get("rss_mb", 0),
            row.get("phase_b", {}).get("rss_mb", 0),
        ),
        "power_avg_w": row.get("power_avg_w", None),
        "ops_per_watt": row.get("ops_per_watt", None),
    }
    return summary


# ── Seed brain.db ─────────────────────────────────────────────────────────────

def seed_brain(rows: list[dict]) -> int:
    ts = datetime.now().strftime("%Y%m%d_%H%M%S")
    pushed = 0
    for row in rows:
        summary = extract_summary(row)
        eng = summary["engine"]
        profile = summary["profile"]
        title = f"bench_{eng}_{profile}_{ts}"
        body = json.dumps({
            "timestamp": ts,
            "summary": summary,
            "raw": row,
        }, indent=2)
        if synx_put(title, body):
            print(f"[trend] Seeded: {title}")
            pushed += 1
        else:
            print(f"[trend] FAILED: {title}")
    return pushed


# ── Compare last N runs ───────────────────────────────────────────────────────

def parse_bench_doc(doc: dict) -> dict | None:
    """Extract structured data from a brain.db document."""
    # Try body field first
    body = doc.get("body") or doc.get("text") or doc.get("raw", "")
    if not body:
        return None
    # Find JSON block
    try:
        # body may be the JSON itself
        return json.loads(body)
    except json.JSONDecodeError:
        pass
    # Try to find embedded JSON
    m = re.search(r'\{.*\}', body, re.DOTALL)
    if m:
        try:
            return json.loads(m.group())
        except json.JSONDecodeError:
            pass
    return None


def compare_last_n(n: int):
    """Show delta table for last N runs per engine."""
    rows = load_results()
    engines = sorted(set(r.get("engine", r["_engine"]) for r in rows))

    print(f"\n=== Trend: last {n} runs from Synapse brain.db ===\n")

    KEY_METRICS = ["phase_a_ops_sec", "phase_b_ops_sec", "recall_at_10", "p50_ms", "rss_mb"]

    for eng in engines:
        docs = synx_search(f"bench_{eng}", limit=n * 2)
        bench_docs = []
        for d in docs:
            parsed = parse_bench_doc(d)
            if parsed and parsed.get("summary", {}).get("engine") == eng:
                bench_docs.append(parsed)

        if not bench_docs:
            print(f"[{eng}] No historical data in brain.db yet.")
            continue

        # Sort by timestamp
        bench_docs.sort(key=lambda x: x.get("timestamp", ""), reverse=True)
        bench_docs = bench_docs[:n]

        print(f"[{eng}] Last {len(bench_docs)} run(s):")
        print(f"  {'Metric':<25} {'Current':>12} {'Prev':>12} {'Delta%':>10}")
        print(f"  {'-'*60}")

        current_summary = extract_summary(next(r for r in rows if r.get("engine", r["_engine"]) == eng))
        prev_summary = bench_docs[0]["summary"] if bench_docs else {}

        for metric in KEY_METRICS:
            cur = current_summary.get(metric)
            prev = prev_summary.get(metric)
            if cur is None and prev is None:
                continue
            cur_s = f"{cur:.2f}" if cur is not None else "N/A"
            prev_s = f"{prev:.2f}" if prev is not None else "N/A"
            if cur is not None and prev is not None and prev != 0:
                delta = ((cur - prev) / abs(prev)) * 100
                delta_s = f"{delta:+.1f}%"
                if delta < -10:
                    delta_s = f"[REGRESSION] {delta_s}"
            else:
                delta_s = "N/A"
            print(f"  {metric:<25} {cur_s:>12} {prev_s:>12} {delta_s:>10}")
        print()


# ── Regression check ──────────────────────────────────────────────────────────

def regression_check(threshold_pct: float = 10.0) -> int:
    """Return 1 if any metric dropped >threshold% vs previous run."""
    rows = load_results()
    engines = sorted(set(r.get("engine", r["_engine"]) for r in rows))

    KEY_METRICS = ["phase_a_ops_sec", "phase_b_ops_sec", "recall_at_10"]
    regressions = []

    for eng in engines:
        docs = synx_search(f"bench_{eng}", limit=4)
        bench_docs = [parse_bench_doc(d) for d in docs]
        bench_docs = [d for d in bench_docs if d and d.get("summary", {}).get("engine") == eng]
        if not bench_docs:
            continue
        bench_docs.sort(key=lambda x: x.get("timestamp", ""), reverse=True)
        prev_summary = bench_docs[0]["summary"]

        current_summary = extract_summary(next(r for r in rows if r.get("engine", r["_engine"]) == eng))

        for metric in KEY_METRICS:
            cur = current_summary.get(metric)
            prev = prev_summary.get(metric)
            if cur is None or prev is None or prev == 0:
                continue
            # For latency metrics, regression = increase
            is_latency = "ms" in metric
            delta = ((cur - prev) / abs(prev)) * 100
            if is_latency:
                delta = -delta  # invert: higher latency = worse

            if delta < -threshold_pct:
                regressions.append((eng, metric, cur, prev, delta))

    if regressions:
        print(f"\n[trend] REGRESSION DETECTED ({len(regressions)} metric(s) dropped >{threshold_pct}%):")
        for eng, metric, cur, prev, delta in regressions:
            print(f"  {eng}.{metric}: {prev:.2f} → {cur:.2f} ({delta:+.1f}%)")
        return 1
    else:
        print(f"[trend] No regressions detected (threshold: {threshold_pct}%)")
        return 0


# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    ap = argparse.ArgumentParser(description="Synapse bench trend tracker")
    ap.add_argument("--compare", metavar="last:N", help="Show delta table for last N runs (e.g. last:5)")
    ap.add_argument("--regression-check", action="store_true", help="Exit 1 if >10%% regression")
    ap.add_argument("--threshold", type=float, default=10.0, help="Regression threshold %% (default 10)")
    ap.add_argument("--seed", action="store_true", default=False, help="Force seed even without --compare/--regression-check")
    args = ap.parse_args()

    rows = load_results()
    if not rows:
        print("[trend] No results found. Run bench first.", file=sys.stderr)
        sys.exit(1)

    if args.compare:
        n = int(args.compare.split(":")[-1]) if ":" in args.compare else int(args.compare)
        # Seed current results first
        pushed = seed_brain(rows)
        print(f"[trend] Seeded {pushed} docs to brain.db")
        compare_last_n(n)
        return

    if args.regression_check:
        # Seed then check
        pushed = seed_brain(rows)
        print(f"[trend] Seeded {pushed} docs")
        code = regression_check(args.threshold)
        sys.exit(code)

    # Default: just seed
    pushed = seed_brain(rows)
    print(f"[trend] Seeded {pushed} docs to Synapse brain.db")
    print("[trend] Run with --compare last:5 to see trends")
    print("[trend] Run with --regression-check for CI gate")


if __name__ == "__main__":
    main()
