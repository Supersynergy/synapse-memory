#!/usr/bin/env python3
"""Reads JSONL results, generates RESULTS.md with markdown tables + analysis."""
import os, json, glob
from datetime import datetime

DIR = os.path.dirname(os.path.abspath(__file__))
RESULTS_DIR = os.path.join(DIR, "results")
OUT = os.path.join(DIR, "RESULTS.md")


def load_latest():
    # Prefer full per-engine files over dry
    for suffix in ("full", "dry"):
        files = glob.glob(os.path.join(RESULTS_DIR, f"*_{suffix}.jsonl"))
        files = [f for f in files if "summary_" not in os.path.basename(f)]
        if files:
            results = {}
            for fpath in sorted(files):
                with open(fpath) as f:
                    for line in f:
                        if line.strip():
                            r = json.loads(line)
                            results[r["engine"]] = r  # last entry wins
            return list(results.values()), suffix
    return [], "none"


def fmt(v, decimals=0):
    if v is None or v == 0:
        return "—"
    if decimals == 0:
        return f"{v:,.0f}"
    return f"{v:.{decimals}f}"


def main():
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--partial", action="store_true",
                        help="Generate partial/in-progress report from whatever JSONL is available")
    parser.add_argument("--out", default=None, help="Output path (default: RESULTS.md or RESULTS_INTERIM.md)")
    args = parser.parse_args()

    results, run_type = load_latest()
    if not results:
        print("No results found. Run bench.py first.")
        return

    global OUT
    if args.partial:
        OUT = os.path.join(DIR, "RESULTS_INTERIM.md")
    if args.out:
        OUT = args.out

    lines = [
        f"# Synapse Comprehensive Benchmark Results",
        f"> Generated: {datetime.now().strftime('%Y-%m-%d %H:%M')}  |  Run type: **{run_type}**",
        "",
    ]

    # Phase A table
    lines += [
        "## Phase A — Bulk Insert",
        "",
        "| Engine | ops/sec | Elapsed (s) | Disk MB | RSS MB | CPU% |",
        "|--------|--------:|------------:|--------:|-------:|-----:|",
    ]
    for r in results:
        a = r.get("phase_a", {})
        err = r.get("error", "")
        if err and not a:
            lines.append(f"| {r['engine']} | ERR | — | — | — | — |")
            continue
        lines.append(f"| {r['engine']} | {fmt(a.get('ops_sec'))} | {fmt(a.get('elapsed_s'),1)} | "
                     f"{fmt(a.get('disk_mb'),1)} | {fmt(a.get('rss_mb'),0)} | {fmt(a.get('cpu_pct_mean'),0)} |")

    lines += [""]

    # Phase B table
    lines += [
        "## Phase B — Update (random 1k rows)",
        "",
        "| Engine | ops/sec | Elapsed (s) | RSS MB | CPU% |",
        "|--------|--------:|------------:|-------:|-----:|",
    ]
    for r in results:
        b = r.get("phase_b", {})
        if not b:
            lines.append(f"| {r['engine']} | — | — | — | — |")
            continue
        lines.append(f"| {r['engine']} | {fmt(b.get('ops_sec'))} | {fmt(b.get('elapsed_s'),2)} | "
                     f"{fmt(b.get('rss_mb'),0)} | {fmt(b.get('cpu_pct_mean'),0)} |")

    lines += [""]

    # Phase C table
    lines += [
        "## Phase C — Mixed 80/20 Read/Write (60s)",
        "",
        "| Engine | ops/sec | p50 ms | p95 ms | p99 ms |",
        "|--------|--------:|-------:|-------:|-------:|",
    ]
    for r in results:
        c = r.get("phase_c", {})
        if not c:
            lines.append(f"| {r['engine']} | — | — | — | — |")
            continue
        lines.append(f"| {r['engine']} | {fmt(c.get('ops_sec'),1)} | {fmt(c.get('p50_ms'),1)} | "
                     f"{fmt(c.get('p95_ms'),1)} | {fmt(c.get('p99_ms'),1)} |")

    lines += [""]

    # Phase D table
    lines += [
        "## Phase D — 8-Thread Concurrent Select",
        "",
        "| Engine | ops/sec | p50 ms | p95 ms | p99 ms |",
        "|--------|--------:|-------:|-------:|-------:|",
    ]
    for r in results:
        d = r.get("phase_d", {})
        if not d:
            lines.append(f"| {r['engine']} | — | — | — | — |")
            continue
        lines.append(f"| {r['engine']} | {fmt(d.get('ops_sec'),1)} | {fmt(d.get('p50_ms'),1)} | "
                     f"{fmt(d.get('p95_ms'),1)} | {fmt(d.get('p99_ms'),1)} |")

    lines += [""]

    # Overhead analysis
    lines += [
        "## Overhead Analysis (CPU + RAM per 1k ops)",
        "",
        "| Engine | RAM/1k-inserts (MB) | CPU%-insert | p99-concurrent (ms) |",
        "|--------|--------------------:|------------:|--------------------:|",
    ]
    for r in results:
        a = r.get("phase_a", {})
        d = r.get("phase_d", {})
        n = r.get("n_docs", 1)
        rss_per_k = a.get("rss_mb", 0) / (n / 1000) if n else 0
        lines.append(f"| {r['engine']} | {fmt(rss_per_k,2)} | {fmt(a.get('cpu_pct_mean'),0)} | "
                     f"{fmt(d.get('p99_ms'),1)} |")

    lines += [""]

    # Errors section
    errors = [(r["engine"], r.get("error", "")) for r in results if r.get("error") and not r.get("phase_a")]
    if errors:
        lines += ["## Errors", ""]
        for eng, err in errors:
            lines.append(f"- **{eng}**: `{err}`")
        lines += [""]

    # Extended phases if present
    extended_engines = [r for r in results if r.get("phase_e") or r.get("phase_f") or r.get("phase_g")]
    if extended_engines:
        lines += [
            "## Phase E — Recall@10 (vs brute-force cosine)",
            "",
            "| Engine | Recall@10 | Queries |",
            "|--------|----------:|--------:|",
        ]
        for r in results:
            e = r.get("phase_e", {})
            if not e:
                lines.append(f"| {r['engine']} | — | — |")
                continue
            lines.append(f"| {r['engine']} | {e.get('recall_at_10', 0):.3f} | {e.get('n_queries', 0)} |")
        lines += [""]

        lines += [
            "## Phase F — Concurrency Sweep (ops/sec vs threads)",
            "",
            "| Engine | 1T ops/s | 4T ops/s | 8T ops/s | 16T ops/s | 16T p99ms |",
            "|--------|----------:|---------:|---------:|----------:|----------:|",
        ]
        for r in results:
            f_data = r.get("phase_f", [])
            if not f_data:
                lines.append(f"| {r['engine']} | — | — | — | — | — |")
                continue
            by_t = {x["threads"]: x for x in f_data}
            def g(t): return fmt(by_t.get(t, {}).get("ops_sec"), 0)
            p99_16 = fmt(by_t.get(16, {}).get("p99_ms"), 1)
            lines.append(f"| {r['engine']} | {g(1)} | {g(4)} | {g(8)} | {g(16)} | {p99_16} |")
        lines += [""]

        lines += [
            "## Phase G — Batch Update Sweep (ops/sec vs batch size)",
            "",
            "| Engine | batch=1 | batch=100 | batch=1000 |",
            "|--------|--------:|----------:|-----------:|",
        ]
        for r in results:
            g_data = r.get("phase_g", [])
            if not g_data:
                lines.append(f"| {r['engine']} | — | — | — |")
                continue
            by_b = {x["batch_size"]: x for x in g_data}
            def gb(b): return fmt(by_b.get(b, {}).get("ops_sec"), 0)
            lines.append(f"| {r['engine']} | {gb(1)} | {gb(100)} | {gb(1000)} |")
        lines += [""]

        soak_engines = [r for r in results if r.get("phase_h")]
        if soak_engines:
            lines += [
                "## Phase H — Soak Test (3 min, 80/20 mixed)",
                "",
                "| Engine | ops/s start | ops/s end | drift% | RSS growth MB |",
                "|--------|------------:|----------:|-------:|--------------:|",
            ]
            for r in results:
                h = r.get("phase_h", {})
                if not h or "error" in h:
                    lines.append(f"| {r['engine']} | — | — | — | — |")
                    continue
                lines.append(f"| {r['engine']} | {fmt(h.get('ops_first'),1)} | {fmt(h.get('ops_last'),1)} | "
                             f"{fmt(h.get('ops_drift_pct'),1)} | {fmt(h.get('rss_growth_mb'),0)} |")
            lines += [""]

    lines += [
        "## How to re-run",
        "",
        "```bash",
        "# Dry run (1k docs, ~2 min)",
        "DRY_RUN=1 bash bench/comprehensive/run.sh",
        "",
        "# Full run with all extended phases (100k docs)",
        "PHASES=all setsid nohup bash bench/comprehensive/run.sh > bench/comprehensive/run.log 2>&1 &",
        "tail -f bench/comprehensive/run.log",
        "```",
    ]

    content = "\n".join(lines)
    with open(OUT, "w") as f:
        f.write(content + "\n")
    print(f"[report] Written: {OUT}")
    print(content)


if __name__ == "__main__":
    main()
