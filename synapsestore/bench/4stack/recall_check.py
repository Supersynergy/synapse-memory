#!/usr/bin/env python3
"""Compute recall@10: top-10 ID overlap vs Stack A (sqlite-vec baseline)."""
import json, sys
from pathlib import Path


def recall_at_k(baseline_ids: dict, other_ids: dict, k: int = 10) -> float:
    scores = []
    for q in baseline_ids:
        base = set(baseline_ids[q][:k])
        other = set(other_ids.get(q, [])[:k])
        if not base:
            continue
        scores.append(len(base & other) / len(base))
    if not scores:
        return 0.0
    return sum(scores) / len(scores)


def compute(results_a: dict, results_x: dict) -> dict:
    if not results_a.get("available") or not results_x.get("available"):
        return {"recall_at_10": None, "note": "stack unavailable"}
    base = results_a.get("baseline_ids", {})
    other = results_x.get("baseline_ids", {})
    r = recall_at_k(base, other)
    return {"recall_at_10": round(r, 4)}


if __name__ == "__main__":
    import argparse
    p = argparse.ArgumentParser()
    p.add_argument("baseline_json")
    p.add_argument("other_json")
    args = p.parse_args()
    base = json.loads(Path(args.baseline_json).read_text())
    other = json.loads(Path(args.other_json).read_text())
    print(json.dumps(compute(base, other), indent=2))
