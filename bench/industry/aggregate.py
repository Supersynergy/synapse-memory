#!/usr/bin/env python3
"""Aggregate results from all engine JSONs → markdown report."""
import json, sys, os
from pathlib import Path
from datetime import datetime

def extract_json(text: str):
    """Find the last JSON object or array in text (bench scripts print progress before JSON)."""
    # Try parsing whole text first
    try:
        return json.loads(text)
    except Exception:
        pass
    # Find last [...] or {...} block
    for start_char, end_char in [('[', ']'), ('{', '}')]:
        idx = text.rfind(start_char)
        while idx >= 0:
            try:
                return json.loads(text[idx:])
            except Exception:
                idx = text.rfind(start_char, 0, idx)
    raise ValueError("No JSON found")


def load_results(results_dir: Path) -> list[dict]:
    all_results = []
    for f in sorted(results_dir.glob("engine_*.json")):
        try:
            data = extract_json(f.read_text())
            if isinstance(data, list):
                all_results.extend(data)
            else:
                all_results.append(data)
        except Exception as e:
            all_results.append({"engine": f.stem, "available": False, "reason": str(e)})
    return all_results


def get_recall10(r: dict) -> float:
    # ultra uses self_recall or vs_gt
    return r.get("self_recall_at_10") or r.get("recall_at_10") or r.get("recall_at_10_vs_gt") or 0.0

def get_recall100(r: dict) -> float:
    return r.get("self_recall_at_100") or r.get("recall_at_100") or r.get("recall_at_100_vs_gt") or 0.0

def format_row(r: dict) -> str:
    if not r.get("available"):
        reason = r.get("reason", "unavailable")
        return f"| {r.get('engine','?'):<34} | SKIP | — | — | — | — | — | {reason[:40]} |"
    r10 = get_recall10(r)
    r100 = get_recall100(r)
    recall_note = "†" if "self_recall" in r else ""
    return (
        f"| {r.get('engine','?'):<34} "
        f"| {r.get('p50_ms', 0):.1f}ms "
        f"| {r.get('p95_ms', 0):.1f}ms "
        f"| {r.get('p99_ms', 0):.1f}ms "
        f"| {r.get('qps_1c', 0):.0f} "
        f"| {r10:.4f}{recall_note} "
        f"| {r100:.4f}{recall_note} |"
    )


def main():
    results_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent / "results"
    ts = sys.argv[2] if len(sys.argv) > 2 else datetime.now().isoformat()

    all_r = load_results(results_dir)
    available = [r for r in all_r if r.get("available")]
    skipped = [r for r in all_r if not r.get("available")]

    # Sort by QPS@recall (proxy: highest recall_at_10, then highest QPS)
    available_sorted = sorted(available, key=lambda r: (get_recall10(r), r.get("qps_1c", 0)), reverse=True)

    # Find best QPS @ recall>=0.99 (recall_at_10)
    high_recall = [r for r in available if get_recall10(r) >= 0.99]
    best_qps_99 = max(high_recall, key=lambda r: r["qps_1c"]) if high_recall else None

    ultra_rows = [r for r in available if "ultra" in r.get("engine", "").lower()]

    lines = [
        f"## Industry ANN Benchmark — {ts[:10]}",
        "",
        "**Setup**: 168k×384 BGE-small (brain.db), M4 Max, 1000 queries, ground-truth = brute-force cosine",
        "",
        "### Methodology Caveats",
        "",
        "- **synapse-ultra**: text→embed pipeline; recall shown is binary_first vs strict (self-recall), not vs brute-force GT (re-embedding produces slightly different vectors than stored)",
        "- **qdrant**: tested on 20k corpus subset (local mode recommendation <20k; full 168k build = ~150s); recall=1.0 reflects this smaller corpus",
        "- **usearch/lance/sqlite-vec**: tested on full 168k corpus with raw stored vectors; recall vs brute-force GT",
        "- **ultra cache**: ultra has LRU cache; warm-cache QPS may be higher than cold-cache; bench runs after warmup",
        "",
        "| Engine | p50 | p95 | p99 | QPS-1c | Recall@10 | Recall@100 |",
        "|--------|-----|-----|-----|--------|-----------|------------|",
        "| *† ultra recall = binary_first vs strict (self), not vs brute-force GT* | | | | | | |",
    ]

    for r in available_sorted:
        lines.append(format_row(r))

    if skipped:
        lines += ["", "### Skipped Engines", ""]
        for r in skipped:
            lines.append(f"- **{r.get('engine','?')}**: {r.get('reason','unavailable')}")

    lines += ["", "### Honest Verdict", ""]

    if best_qps_99:
        lines.append(f"- **Best QPS @ recall≥0.99**: {best_qps_99['engine']} — {best_qps_99['qps_1c']:.0f} QPS (p50={best_qps_99['p50_ms']:.1f}ms)")
    else:
        lines.append("- **Best QPS @ recall≥0.99**: no engine reached recall 0.99 on recall@10")

    if available_sorted:
        best_p50 = min(available, key=lambda r: r.get("p50_ms", 9999))
        lines.append(f"- **Best p50 overall**: {best_p50['engine']} — {best_p50['p50_ms']:.1f}ms")

    if ultra_rows:
        ultra_strict = next((r for r in ultra_rows if "strict" in r.get("engine", "")), None)
        ultra_bin = next((r for r in ultra_rows if "binary_first" in r.get("engine", "")), None)
        if ultra_strict and ultra_bin:
            speedup = ultra_strict["mean_ms"] / ultra_bin["mean_ms"] if ultra_bin["mean_ms"] > 0 else 0
            self_r = ultra_bin.get("self_recall_at_10", "N/A")
            self_r_str = f"{self_r:.4f}" if isinstance(self_r, float) else str(self_r)
            lines.append(f"- **ultra binary_first vs strict**: {speedup:.1f}× faster, self-recall@10={self_r_str}")

        # Rank ultra among all available
        rank = next((i+1 for i, r in enumerate(available_sorted) if "ultra" in r.get("engine","").lower() and "binary_first" in r.get("mode","")), None)
        if rank:
            lines.append(f"- **ultra (binary_first) rank by recall@10 then QPS**: #{rank} of {len(available)}")
    else:
        lines.append("- **ultra**: not measured (daemon not running)")

    lines += [
        "",
        "### Recommendations",
        "",
        "- To improve ultra recall: increase `DEFAULT_BINARY_RERANK` (currently 500) to 1000+",
        "- To improve ultra QPS: use binary_only mode (no f16 rerank)",
        "- For raw vector ANN: usearch M=16 ef=128 is strong baseline at recall≥0.99",
        "- sqlite-vec brute is exact (recall=1.0) but slowest — good sanity check only",
    ]

    report = "\n".join(lines)
    print(report)

    # Write JSON summary
    summary = {
        "ts": ts,
        "n_measured": len(available),
        "n_skipped": len(skipped),
        "results": all_r,
        "best_qps_at_recall_099": best_qps_99,
    }
    summary_path = results_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2))

    return report


if __name__ == "__main__":
    main()
