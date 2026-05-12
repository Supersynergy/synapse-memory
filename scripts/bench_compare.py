#!/usr/bin/env python3
"""
Parse criterion JSON outputs from two dirs, compare, post PR comment.
Regression threshold: relative increase in mean estimate (lower is faster).
"""
import argparse
import json
import os
import pathlib
import sys
import urllib.request


def load_estimates(criterion_dir: pathlib.Path) -> dict[str, float]:
    results = {}
    if not criterion_dir.exists():
        return results
    for est_file in criterion_dir.rglob("new/estimates.json"):
        # path: <criterion_dir>/<bench_name>/<group>/new/estimates.json
        parts = est_file.parts
        # bench name = two levels above "new"
        bench_key = "/".join(parts[len(criterion_dir.parts):-2])
        try:
            data = json.loads(est_file.read_text())
            mean_ns = data["mean"]["point_estimate"]
            results[bench_key] = mean_ns
        except (KeyError, json.JSONDecodeError):
            continue
    return results


def load_baselines(path: pathlib.Path) -> dict[str, float]:
    if path.exists():
        try:
            return json.loads(path.read_text())
        except json.JSONDecodeError:
            pass
    return {}


def format_delta(delta: float) -> str:
    sign = "+" if delta >= 0 else ""
    return f"{sign}{delta*100:.1f}%"


def build_comment(rows: list[dict], regressed: list[str], threshold: float) -> str:
    header = "## Bench Regression Report\n\n"
    if not rows:
        return header + "_No criterion benchmarks found in either branch._\n"

    table = "| Benchmark | Base (ns) | PR (ns) | Delta |\n"
    table += "|-----------|----------:|--------:|-------|\n"
    for r in rows:
        flag = " ⚠️" if r["key"] in regressed else ""
        table += f"| `{r['key']}`{flag} | {r['base']:,.0f} | {r['pr']:,.0f} | {format_delta(r['delta'])} |\n"

    if regressed:
        summary = (
            f"\n**❌ {len(regressed)} regression(s) exceed {threshold*100:.0f}% threshold — failing.**\n"
        )
    else:
        summary = f"\n**✅ All benchmarks within {threshold*100:.0f}% threshold.**\n"

    return header + table + summary


def post_comment(repo: str, pr: int, token: str, body: str) -> None:
    url = f"https://api.github.com/repos/{repo}/issues/{pr}/comments"
    payload = json.dumps({"body": body}).encode()
    req = urllib.request.Request(
        url,
        data=payload,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/vnd.github+json",
        },
        method="POST",
    )
    with urllib.request.urlopen(req) as resp:
        if resp.status not in (200, 201):
            print(f"GitHub comment failed: {resp.status}", file=sys.stderr)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pr-dir", required=True)
    ap.add_argument("--base-dir", required=True)
    ap.add_argument("--baselines", required=True)
    ap.add_argument("--threshold", type=float, default=0.10)
    ap.add_argument("--pr", type=int, required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--token", required=True)
    args = ap.parse_args()

    pr_data = load_estimates(pathlib.Path(args.pr_dir))
    base_data = load_estimates(pathlib.Path(args.base_dir))
    fallback = load_baselines(pathlib.Path(args.baselines))

    # merge: prefer live base_data, fall back to committed baselines
    combined_base = {**fallback, **base_data}

    rows = []
    regressed = []

    all_keys = sorted(set(pr_data) | set(combined_base))
    for key in all_keys:
        if key not in pr_data or key not in combined_base:
            continue
        base_ns = combined_base[key]
        pr_ns = pr_data[key]
        delta = (pr_ns - base_ns) / base_ns if base_ns else 0.0
        rows.append({"key": key, "base": base_ns, "pr": pr_ns, "delta": delta})
        if delta > args.threshold:
            regressed.append(key)

    comment = build_comment(rows, regressed, args.threshold)
    print(comment)

    post_comment(args.repo, args.pr, args.token, comment)

    if regressed:
        print(f"\nREGRESSION: {regressed}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
