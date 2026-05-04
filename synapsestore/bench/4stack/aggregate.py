#!/usr/bin/env python3
"""Read per-stack JSONs, compute recall, output markdown comparison table."""
import json, sys, argparse, datetime
from pathlib import Path

STACK_ORDER = ["A", "B", "C", "D"]
LABELS = {
    "A": "sqlite-vec brute-force",
    "B": "Python turbo :9477",
    "C": "Core+ndarray (Step-5)",
    "D": "synapse-ultra",
}


def fmt(v, decimals=2):
    if v is None:
        return "-"
    return f"{v:.{decimals}f}"


def recall_at_k(base: dict, other: dict, k=10) -> float | None:
    if not base or not other:
        return None
    scores = []
    for q in base:
        b = set(base[q][:k])
        o = set(other.get(q, [])[:k])
        if b:
            scores.append(len(b & o) / len(b))
    return round(sum(scores) / len(scores), 4) if scores else None


def run(result_dir: Path, ts: str) -> str:
    results = {}
    for stack in STACK_ORDER:
        p = result_dir / f"stack_{stack}.json"
        if p.exists():
            results[stack] = json.loads(p.read_text())

    if not results:
        return "# No results found\n"

    baseline_ids = results.get("A", {}).get("baseline_ids", {})

    rows = []
    for s in STACK_ORDER:
        r = results.get(s)
        if r is None:
            rows.append((s, LABELS[s], "⚠️ no data", "-", "-", "-", "-", "-", "-"))
            continue

        if not r.get("available"):
            reason = r.get("reason", "unknown")
            rows.append((s, LABELS[s], f"⚠️ skipped ({reason})", "-", "-", "-", "-", "-", "-"))
            continue

        cold = r.get("cold", {})
        warm = r.get("warm", {})

        # Recall vs baseline A
        if s == "A":
            recall = "1.0000 (baseline)"
        else:
            other_ids = r.get("baseline_ids", {})
            rv = recall_at_k(baseline_ids, other_ids)
            recall = f"{rv:.4f}" if rv is not None else "n/a"

        rows.append((
            s,
            LABELS.get(s, r.get("label", s)),
            "✅",
            fmt(cold.get("p50")),
            fmt(cold.get("p95")),
            fmt(cold.get("p99")),
            fmt(warm.get("p50")),
            f"{warm.get('qps', 0):.0f}",
            recall,
        ))

    # Build brain.db info
    db_path = Path.home() / ".synapse" / "brain.db"
    db_info = f"{db_path}" if db_path.exists() else "unknown"

    lines = [
        f"# 4-Stack Vector Bench — {ts}",
        f"## Setup: brain.db at {db_info}, M4 Max, queries=100",
        "",
        "| Stack | Label | Available | cold p50 | cold p95 | cold p99 | warm p50 | warm QPS | Recall@10 |",
        "|-------|-------|-----------|----------|----------|----------|----------|----------|-----------|",
    ]
    for row in rows:
        s, label, avail, cp50, cp95, cp99, wp50, qps, recall = row
        lines.append(f"| {s} | {label} | {avail} | {cp50} ms | {cp95} ms | {cp99} ms | {wp50} ms | {qps} | {recall} |")

    lines += [
        "",
        "## Caveats",
    ]

    for s in STACK_ORDER:
        r = results.get(s)
        if r and r.get("available"):
            cold_mean = r.get("cold", {}).get("mean")
            warm_mean = r.get("warm", {}).get("mean")
            if cold_mean and warm_mean and warm_mean > cold_mean:
                lines.append(f"- ⚠️ Stack {s}: warm mean ({warm_mean:.2f}ms) > cold mean ({cold_mean:.2f}ms) — anomalous, investigate embed cache behavior")

    # Flag if A > B in QPS (B should be faster warm)
    a_warm = results.get("A", {}).get("warm", {}).get("qps", 0)
    b_warm = results.get("B", {}).get("warm", {}).get("qps", 0)
    if a_warm and b_warm and a_warm > b_warm and results.get("B", {}).get("available"):
        lines.append(f"- ⚠️ ANOMALY: Stack A QPS ({a_warm:.0f}) > Stack B QPS ({b_warm:.0f}) — unexpected; B (NumPy) should be faster warm. Investigate network overhead vs socket.")

    lines += [
        "",
        "## Notes",
        f"- Cold = 1 pass × {results.get('A', {}).get('n_queries_cold', 100)} queries",
        f"- Warm = {results.get('A', {}).get('n_iters_warm', 100)} iterations on first query (embed cache hot)",
        "- Recall@10 = mean top-10 ID overlap vs Stack A sqlite-vec baseline",
        "- C and D skipped when build/daemon absent (not a failure)",
        "",
    ]

    return "\n".join(lines)


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("result_dir")
    p.add_argument("--ts", default=datetime.datetime.now().strftime("%Y-%m-%dT%H:%M:%S"))
    args = p.parse_args()

    result_dir = Path(args.result_dir)
    md = run(result_dir, args.ts)
    print(md)

    # Write report
    report_path = result_dir / f"run-{args.ts.replace(':', '-')}.md"
    report_path.write_text(md)
    print(f"\n[report written to {report_path}]", file=sys.stderr)
