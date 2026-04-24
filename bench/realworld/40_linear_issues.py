"""Linear issues bench — b40.

Expects a Linear GraphQL issues dump as JSON. Parses both standard API format
(top-level array) and nested GraphQL format (data.issues.nodes).

For each issue, concatenates title + "\\n" + description, handles nulls,
truncates at 4000 chars, then runs the generic harness.

Usage:
    python 40_linear_issues.py <issues.json> [max_issues]
"""

from __future__ import annotations
import json
import math
import random
import statistics
import sys
import time
from pathlib import Path

import synapse

from harness import HashEmbedder


def flatten(issues_data: list | dict, max_issues: int | None = None) -> list[tuple[str, str]]:
    """Return (issue_id, text) from Linear issues.

    Handles both formats:
    - Direct list: [{id, title, description, ...}, ...]
    - GraphQL nested: {data: {issues: {nodes: [...]}}}

    Skips issues where both title and description are empty.
    Truncates combined text at 4000 chars.
    """
    out: list[tuple[str, str]] = []

    # Detect and extract issues list
    if isinstance(issues_data, dict):
        # Try GraphQL nested path
        issues = issues_data.get("data", {}).get("issues", {}).get("nodes", [])
        if not issues:
            # Fallback: try direct top-level issues key
            issues = issues_data.get("issues", [])
    else:
        # Assume direct list
        issues = issues_data

    for issue in issues:
        if not isinstance(issue, dict):
            continue

        issue_id = issue.get("id", "unknown")
        title = issue.get("title", "").strip()
        description = issue.get("description", "").strip()

        # Skip if both are empty
        if not title and not description:
            continue

        # Concat with newline separator
        text = (title + "\n" + description) if title and description else (title or description)

        # Truncate at 4000 chars
        text = text[:4000]

        out.append((str(issue_id), text))

        if max_issues and len(out) >= max_issues:
            break

    return out


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("Usage: python 40_linear_issues.py <issues.json> [max_issues]", file=sys.stderr)
        return 1

    path = Path(argv[1]).expanduser().resolve()
    max_issues = int(argv[2]) if len(argv) > 2 else None

    try:
        data = json.loads(path.read_text())
    except Exception as e:
        print(f"failed to parse {path.name}: {e}", file=sys.stderr)
        return 1

    issues = flatten(data, max_issues=max_issues)
    if not issues:
        print("no issues parsed", file=sys.stderr)
        return 1

    print(f"▸ {len(issues):,} issues flattened from {path.name}")

    emb = HashEmbedder(dim=128)
    embs = emb.embed_documents([t for _, t in issues])
    rows = [(i, v) for i, v in enumerate(embs)]

    t0 = time.perf_counter()
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    t_build = (time.perf_counter() - t0) * 1000

    # 20 random issues as queries — simulates "find the dup before you file it"
    random.seed(8)
    queries = [issues[random.randrange(len(issues))][1][:120] for _ in range(20)]

    durs = []
    for q in queries:
        vec = emb.embed_query(q)
        t = time.perf_counter()
        _ = synapse.rerank(ham, i8, vec, k=10, candidates=80) if len(issues) >= 1000 else i8.search(vec, k=10)
        durs.append((time.perf_counter() - t) * 1e6)

    ds = sorted(durs)
    def pct(p: float) -> float:
        return ds[min(len(ds) - 1, int(p * len(ds)))]

    print(f"▸ build indices          {t_build:>7.1f} ms")
    print(f"▸ p50                    {statistics.median(durs):>7.0f} µs")
    print(f"▸ p95                    {pct(0.95):>7.0f} µs")
    print(f"▸ p99                    {pct(0.99):>7.0f} µs")
    print()
    print(f"consumer framing: «{len(issues):,} Linear issues searched in {pct(0.95)/1000:.1f} ms — find the dup before you file it»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
