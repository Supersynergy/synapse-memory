#!/usr/bin/env python3
"""
SuperML CatBoost tuner for Synapse config sweep results.
Reads results.jsonl produced by harness.py, trains CatBoost,
outputs BEST_CONFIG.json + WHY.md with SHAP feature importances.

Usage:
    python tune.py [--results results.jsonl]
"""
from __future__ import annotations

import json
import pathlib
import sys

RESULTS_FILE = pathlib.Path(__file__).parent / "results.jsonl"
BEST_CONFIG_FILE = pathlib.Path(__file__).parent / "BEST_CONFIG.json"
WHY_FILE = pathlib.Path(__file__).parent / "WHY.md"

FEATURES = ["cache_size_mb", "mmap_size_mb", "page_size", "journal_mode", "synchronous", "batch_size"]
TARGET = "insert_ops_s"


def load_results(path: pathlib.Path) -> list[dict]:
    rows = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if line:
            rows.append(json.loads(line))
    return rows


def encode(rows: list[dict]):
    X, y = [], []
    cat_map_journal = {"WAL": 0, "MEMORY": 1}
    cat_map_sync = {"NORMAL": 0, "OFF": 1}
    for r in rows:
        X.append([
            r["cache_size_mb"],
            r["mmap_size_mb"],
            r["page_size"],
            cat_map_journal.get(r["journal_mode"], 0),
            cat_map_sync.get(r["synchronous"], 0),
            r["batch_size"],
        ])
        y.append(r[TARGET])
    return X, y


def best_by_score(rows: list[dict]) -> dict:
    return max(rows, key=lambda r: r["insert_ops_s"] / max(r["query_p50_ms"], 0.001))


def run_catboost(rows: list[dict]) -> tuple[dict, list[tuple[str, float]]]:
    try:
        from catboost import CatBoostRegressor
        import numpy as np
    except ImportError:
        return {}, []

    X, y = encode(rows)
    import numpy as np
    X_np = np.array(X, dtype=float)
    y_np = np.array(y, dtype=float)

    model = CatBoostRegressor(iterations=200, depth=4, learning_rate=0.1, verbose=0)
    model.fit(X_np, y_np)

    importances = list(zip(FEATURES, model.get_feature_importance()))
    importances.sort(key=lambda x: x[1], reverse=True)

    return {}, importances


def main() -> None:
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--results", default=str(RESULTS_FILE))
    args = parser.parse_args()

    results_path = pathlib.Path(args.results)
    if not results_path.exists():
        print(f"ERROR: {results_path} not found. Run harness.py first.")
        sys.exit(1)

    rows = load_results(results_path)
    print(f"Loaded {len(rows)} result rows.")

    best = best_by_score(rows)
    print(f"\nBest config by heuristic (insert_ops_s / query_p50_ms):")
    for k in FEATURES + ["insert_ops_s", "query_p50_ms", "db_size_kb"]:
        if k in best:
            print(f"  {k}: {best[k]}")

    # Try CatBoost
    importances: list[tuple[str, float]] = []
    try:
        from catboost import CatBoostRegressor
        import numpy as np

        X, y = encode(rows)
        X_np = np.array(X, dtype=float)
        y_np = np.array(y, dtype=float)
        model = CatBoostRegressor(iterations=300, depth=5, learning_rate=0.05, verbose=0)
        model.fit(X_np, y_np)
        importances = sorted(
            zip(FEATURES, model.get_feature_importance()),
            key=lambda x: x[1], reverse=True
        )
        print("\nCatBoost feature importances:")
        for feat, imp in importances:
            print(f"  {feat}: {imp:.2f}%")
    except ImportError:
        print("\nCatBoost not available — using heuristic ranking only.")
        # Rank by variance in insert_ops_s across feature values
        import statistics
        for feat in FEATURES:
            vals = {}
            for r in rows:
                k = r[feat]
                vals.setdefault(k, []).append(r["insert_ops_s"])
            group_means = {k: statistics.mean(v) for k, v in vals.items()}
            spread = max(group_means.values()) - min(group_means.values()) if group_means else 0
            importances.append((feat, round(spread, 2)))
        importances.sort(key=lambda x: x[1], reverse=True)
        print("\nHeuristic feature spread (insert_ops_s range across feature values):")
        for feat, spread in importances:
            print(f"  {feat}: {spread:.1f} ops/s spread")

    # Write BEST_CONFIG.json
    best_out = {k: best[k] for k in FEATURES + ["insert_ops_s", "query_p50_ms", "db_size_kb"] if k in best}
    BEST_CONFIG_FILE.write_text(json.dumps(best_out, indent=2))
    print(f"\nWrote {BEST_CONFIG_FILE}")

    # Write WHY.md
    lines = [
        "# Synapse Auto-Tune — Best Config\n",
        f"**Source**: `{results_path.name}` ({len(rows)} configs)\n",
        "## Best Config\n",
        "```json",
        json.dumps(best_out, indent=2),
        "```\n",
        "## Feature Importance (by impact on insert_ops_s)\n",
        "| Rank | Feature | Importance |",
        "|------|---------|-----------|",
    ]
    for rank, (feat, imp) in enumerate(importances, 1):
        lines.append(f"| {rank} | `{feat}` | {imp} |")
    lines.append("")
    lines.append("## Key Takeaways\n")
    if importances:
        top_feat = importances[0][0]
        lines.append(f"- **`{top_feat}`** is the highest-impact knob.")
    lines.append("- Run `harness.py --full` for the exhaustive 432-point grid.")
    lines.append("- Re-run `tune.py` after expanding dataset or changing workload.")
    WHY_FILE.write_text("\n".join(lines) + "\n")
    print(f"Wrote {WHY_FILE}")


if __name__ == "__main__":
    main()
