#!/usr/bin/env python3
"""Generate RESULTS_FAST.md from fast profile JSONL results."""
import json, os, glob, argparse

DIR = os.path.dirname(os.path.abspath(__file__))
RESULTS_DIR = os.path.join(DIR, "results")


def load_results(suffix="fast"):
    results = {}
    for path in sorted(glob.glob(os.path.join(RESULTS_DIR, f"*_{suffix}.jsonl"))):
        if os.path.basename(path).startswith('summary'):
            continue
        engine = os.path.basename(path).replace(f"_{suffix}.jsonl", "")
        try:
            with open(path) as f:
                data = json.loads(f.read())
            results[engine] = data
        except Exception as e:
            print(f"[warn] Failed to load {path}: {e}")
    return results


def fmt(v, fmt_str=".0f"):
    if v is None:
        return "N/A"
    try:
        return format(v, fmt_str)
    except Exception:
        return str(v)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", default="fast", choices=["fast", "full", "dry"])
    args = parser.parse_args()

    results = load_results(args.profile)
    if not results:
        print(f"No results found for profile={args.profile}")
        return

    engines = sorted(results.keys())
    lines = [f"# Comprehensive Bench Results -- Profile: {args.profile}", ""]

    # Phase A
    lines += ["## Phase A -- Bulk Insert", "",
              "| Engine | ops/sec | RSS MB | Disk MB | CPU% |",
              "|--------|---------|--------|---------|------|"]
    for e in engines:
        d = results[e]
        a = d.get("phase_a", {})
        if "error" in d and "phase_a" not in d:
            lines.append(f"| {e} | ERROR | -- | -- | -- |")
        else:
            lines.append(f"| {e} | {fmt(a.get('ops_sec'))} | {fmt(a.get('rss_mb'))} | {fmt(a.get('disk_mb'),'.1f')} | {fmt(a.get('cpu_pct_mean'),'.0f')} |")
    lines.append("")

    # Phase F
    lines += ["## Phase F -- Concurrency Saturation Sweep", "",
              "| Engine | 1T ops/s | 4T ops/s | 8T ops/s | 16T ops/s | 32T ops/s | 64T ops/s | Sat.Threads |",
              "|--------|----------|----------|----------|-----------|-----------|-----------|-------------|"]
    for e in engines:
        d = results[e]
        fs = d.get("phase_f_saturation", {})
        if fs:
            ops = fs.get("ops_per_s", [])
            sat = fs.get("saturation_threads", "N/A")
            cells = [e] + [fmt(ops[i] if i < len(ops) else None, ".0f") for i in range(6)] + [str(sat)]
            lines.append("| " + " | ".join(cells) + " |")
        else:
            lines.append(f"| {e} | N/A | N/A | N/A | N/A | N/A | N/A | N/A |")
    lines.append("")

    # Phase I
    lines += ["## Phase I -- Cache Hitrate (1001 identical queries)", "",
              "| Engine | first_ms | mean_repeat_ms | p50_ms | p99_ms | speedup |",
              "|--------|----------|----------------|--------|--------|---------|"]
    for e in engines:
        d = results[e]
        ci = d.get("phase_i_cache", {})
        if ci:
            lines.append(f"| {e} | {fmt(ci.get('first_query_ms'),'.3f')} | {fmt(ci.get('mean_repeat_ms'),'.4f')} | {fmt(ci.get('p50_repeat_ms'),'.4f')} | {fmt(ci.get('p99_repeat_ms'),'.4f')} | {fmt(ci.get('speedup_ratio'),'.2f')}x |")
        else:
            lines.append(f"| {e} | N/A | N/A | N/A | N/A | N/A |")
    lines.append("")

    out_path = os.path.join(DIR, f"RESULTS_{args.profile.upper()}.md")
    with open(out_path, "w") as f:
        f.write("\n".join(lines))
    print(f"Written: {out_path}")
    print()
    for l in lines:
        if "Phase F" in l or "Phase I" in l or l.startswith("|") or l == "":
            print(l)


if __name__ == "__main__":
    main()
