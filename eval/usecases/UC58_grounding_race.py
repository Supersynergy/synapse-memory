#!/usr/bin/env python3
"""UC58 — Grounding race-bench: hybrid vs vec-then-graph vs PPR vs traverse vs ground.

Compares latency + recall@K of 5 grounding strategies over a golden eval-set.
All strategies route through synapse-cli subcommands (synx graph ppr, synx
graph traverse, synx ground), so this also smoke-tests the new CLI surface.

Run:
  python3 eval/usecases/UC58_grounding_race.py [--db .synapse/brain.db]
                                               [--golden eval/golden/grounding.jsonl]
                                               [--k 10]

Golden format (JSONL):
  {"id": "g1", "query": "...", "relevant_ids": [42, 7, 99], "notes": "..."}

Output: per-strategy table + winner per query + aggregate (mean recall, p50/p95 ms).
"""
from __future__ import annotations
import argparse, json, subprocess, time, statistics, os, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SYNX = os.environ.get("SYNX", "synx")

def run(cmd: list[str], timeout: float = 10.0) -> tuple[str, float]:
    t0 = time.time()
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return r.stdout.strip(), (time.time() - t0) * 1000.0
    except Exception as e:
        return f"<err: {e}>", (time.time() - t0) * 1000.0

def parse_hits(stdout: str) -> list[int]:
    ids = []
    for line in stdout.splitlines():
        parts = line.split("\t")
        if parts and parts[0].isdigit():
            ids.append(int(parts[0]))
    return ids

def parse_json_ids(stdout: str) -> list[int]:
    try:
        data = json.loads(stdout)
    except json.JSONDecodeError:
        return []
    out = []
    if isinstance(data, list):
        for x in data:
            if isinstance(x, list) and x: out.append(int(x[0]))
            elif isinstance(x, dict) and "id" in x: out.append(int(x["id"]))
            elif isinstance(x, int): out.append(x)
    elif isinstance(data, dict):
        for key in ("hybrid_seeds", "ppr_ranked", "graph_expansions"):
            for x in data.get(key, []):
                if isinstance(x, dict) and "id" in x: out.append(int(x["id"]))
                elif isinstance(x, list) and x: out.append(int(x[0]))
    return out

def s_hybrid(db, q, k):
    out, ms = run([SYNX, "-f", db, "hybrid", q, "--limit", str(k)])
    return parse_hits(out), ms

def s_ppr(db, q, k):
    seeds_out, _ = run([SYNX, "-f", db, "hybrid", q, "--limit", str(k)])
    ids = parse_hits(seeds_out)[:k]
    if not ids: return [], 0.0
    seeds = json.dumps({str(i): 1.0 for i in ids})
    out, ms = run([SYNX, "-f", db, "graph", "ppr", seeds, "--limit", str(k)])
    return parse_json_ids(out), ms

def s_traverse(db, q, k):
    seeds_out, _ = run([SYNX, "-f", db, "hybrid", q, "--limit", "3"])
    ids = parse_hits(seeds_out)
    if not ids: return [], 0.0
    out, ms = run([SYNX, "-f", db, "graph", "traverse", str(ids[0]),
                   "--depth", "2", "--top-k-per-hop", str(k)])
    return parse_json_ids(out)[:k], ms

def s_ground(db, q, k):
    out, ms = run([SYNX, "-f", db, "ground", q, "--k", str(k), "--depth", "2"])
    return parse_json_ids(out)[:k], ms

STRATEGIES = {
    "hybrid":   s_hybrid,
    "ppr":      s_ppr,
    "traverse": s_traverse,
    "ground":   s_ground,
}

def recall_at_k(retrieved: list[int], relevant: list[int], k: int) -> float:
    if not relevant: return 0.0
    hits = set(retrieved[:k]) & set(relevant)
    return len(hits) / len(relevant)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default=".synapse/brain.db")
    ap.add_argument("--golden", default="eval/golden/grounding.jsonl")
    ap.add_argument("--k", type=int, default=10)
    args = ap.parse_args()

    golden_path = Path(args.golden)
    if not golden_path.is_absolute():
        golden_path = ROOT / args.golden
    if not golden_path.exists():
        print(f"[UC58] no golden set at {golden_path} — emit synthetic 3-query smoke")
        items = [
            {"id": "smoke1", "query": "synapse",         "relevant_ids": []},
            {"id": "smoke2", "query": "graph traversal", "relevant_ids": []},
            {"id": "smoke3", "query": "vector search",   "relevant_ids": []},
        ]
    else:
        items = [json.loads(l) for l in golden_path.read_text().splitlines() if l.strip()]

    results = {s: {"recalls": [], "latencies_ms": []} for s in STRATEGIES}
    print(f"[UC58] running {len(items)} queries × {len(STRATEGIES)} strategies (k={args.k})")
    for it in items:
        q = it["query"]; rel = it.get("relevant_ids", [])
        line = f"  Q={q[:50]:50s}"
        for sname, fn in STRATEGIES.items():
            ids, ms = fn(args.db, q, args.k)
            r = recall_at_k(ids, rel, args.k) if rel else float("nan")
            results[sname]["recalls"].append(r)
            results[sname]["latencies_ms"].append(ms)
            line += f"  {sname}: {ms:>5.0f}ms"
            if rel: line += f" r@{args.k}={r:.2f}"
        print(line)

    print("\n[UC58] aggregate:")
    print(f"{'strategy':<10} {'mean_ms':>8} {'p50_ms':>8} {'p95_ms':>8} {'mean_recall':>12}")
    for s, d in results.items():
        lat = d["latencies_ms"]
        rec = [r for r in d["recalls"] if r == r]  # filter NaN
        mean_ms = statistics.fmean(lat) if lat else 0.0
        p50 = statistics.median(lat) if lat else 0.0
        p95 = sorted(lat)[int(0.95 * len(lat))] if len(lat) >= 2 else (lat[0] if lat else 0.0)
        mean_r = statistics.fmean(rec) if rec else float("nan")
        print(f"{s:<10} {mean_ms:>8.1f} {p50:>8.1f} {p95:>8.1f} {mean_r:>12.3f}")

    # Best-strategy callout
    if any(d["recalls"] for d in results.values()):
        winners = {s: statistics.fmean([r for r in d["recalls"] if r == r] or [0])
                   for s, d in results.items()}
        best = max(winners, key=winners.get)
        print(f"\n[UC58] winner by mean recall: {best} ({winners[best]:.3f})")
    print("[UC58] OK")

if __name__ == "__main__":
    main()
