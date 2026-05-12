#!/usr/bin/env python3
"""UC58d — omni multi-criterion grounding score.

Replaces single-recall@K with weighted multi-perspective score:

    omni_score = w_recall * recall@K
               + w_diversity * (1 - mean_pairwise_jaccard(top_K_texts))
               + w_latency * inv_latency_norm
               + w_provenance * provenance_coverage

Each "persona" weights criteria differently:
  - end_user_busy:   recall × latency (cares: fast & relevant)
  - skeptic:         recall × provenance (verifiable sources)
  - investor_seed:   recall × diversity (broad market view)
  - feynman:         recall × diversity (multiple framings of same concept)
  - judge_neutral:   even weights (balanced)

Output: per-persona winner across strategies; consensus via Borda count.

Run:
  python3 eval/usecases/UC58d_omni_score.py --db .synapse/brain.db
                                             --golden eval/golden/grounding.jsonl
"""
from __future__ import annotations
import argparse, json, math, os, statistics, subprocess, time, re
from pathlib import Path
from collections import defaultdict

ROOT = Path(__file__).resolve().parents[2]
SYNX = os.environ.get("SYNX", "synx")

PERSONAS = {
    "end_user_busy":  {"recall":0.5, "latency":0.5, "diversity":0.0, "provenance":0.0},
    "skeptic":        {"recall":0.5, "latency":0.0, "diversity":0.0, "provenance":0.5},
    "investor_seed":  {"recall":0.5, "latency":0.0, "diversity":0.5, "provenance":0.0},
    "feynman":        {"recall":0.4, "latency":0.0, "diversity":0.6, "provenance":0.0},
    "judge_neutral":  {"recall":0.25,"latency":0.25,"diversity":0.25,"provenance":0.25},
}

def tokens(s: str) -> set:
    return set(re.findall(r"\w+", s.lower())) if s else set()

def jaccard(a: set, b: set) -> float:
    if not a or not b: return 0.0
    return len(a & b) / len(a | b)

def diversity_score(texts: list[str]) -> float:
    if len(texts) < 2: return 1.0
    toks = [tokens(t) for t in texts]
    sims = []
    for i in range(len(toks)):
        for j in range(i+1, len(toks)):
            sims.append(jaccard(toks[i], toks[j]))
    return 1.0 - (statistics.fmean(sims) if sims else 0.0)

def parse_hybrid(stdout: str):
    out = []
    for line in stdout.splitlines():
        parts = line.split("\t", 2)
        if len(parts) >= 3 and parts[0].isdigit():
            out.append({"id": int(parts[0]), "score": float(parts[1]),
                        "text": parts[2]})
    return out

def parse_ground(stdout: str):
    try:
        d = json.loads(stdout)
        cands = []
        for h in d.get("hybrid_seeds", []):
            cands.append({"id": int(h["id"]), "score": float(h.get("score",0)),
                          "text": h.get("text",""), "kind":"seed"})
        for x in d.get("graph_expansions", []):
            cands.append({"id": int(x["to_id"]), "score": float(x.get("score",0)),
                          "text":"", "kind":"expansion"})
        for x in d.get("ppr_ranked", []):
            if isinstance(x, list) and len(x) >= 2:
                cands.append({"id": int(x[0]), "score": float(x[1]),
                              "text":"", "kind":"ppr"})
        return cands
    except json.JSONDecodeError: return []

def run_strategy(strat: str, db: str, q: str, k: int):
    t0 = time.time()
    if strat == "hybrid":
        r = subprocess.run([SYNX, "-f", db, "hybrid", q, "--limit", str(k)],
                           capture_output=True, text=True, timeout=15)
        cands = parse_hybrid(r.stdout)
    elif strat == "ground":
        r = subprocess.run([SYNX, "-f", db, "ground", q, "--k", str(k), "--depth", "2"],
                           capture_output=True, text=True, timeout=15)
        cands = parse_ground(r.stdout)
    else: cands = []
    return cands, (time.time() - t0) * 1000.0

def omni_score(criteria: dict, weights: dict) -> float:
    s = 0.0
    for k, w in weights.items():
        s += w * criteria.get(k, 0.0)
    return s

def borda_consensus(rankings: dict[str, list[str]]) -> list[tuple[str, int]]:
    """Per-persona ranking → Borda count."""
    points = defaultdict(int)
    for persona, ranking in rankings.items():
        n = len(ranking)
        for rank, strat in enumerate(ranking):
            points[strat] += (n - rank)
    return sorted(points.items(), key=lambda x: -x[1])

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default=".synapse/brain.db")
    ap.add_argument("--golden", default="eval/golden/grounding.jsonl")
    ap.add_argument("--k", type=int, default=10)
    args = ap.parse_args()

    golden = Path(args.golden) if Path(args.golden).is_absolute() else ROOT / args.golden
    items = []
    if golden.exists():
        items = [json.loads(l) for l in golden.read_text().splitlines() if l.strip()]
    if not items:
        items = [{"id":"smoke","query":"synapse","relevant_ids":[]}]

    strategies = ["hybrid", "ground"]
    persona_score_sums = {p: defaultdict(list) for p in PERSONAS}

    for it in items:
        q, rel = it["query"], it.get("relevant_ids", [])
        for s in strategies:
            cands, ms = run_strategy(s, args.db, q, args.k)
            ids = [c["id"] for c in cands]
            texts = [c.get("text","") for c in cands if c.get("text")]
            recall = (len(set(ids[:args.k]) & set(rel)) / len(rel)) if rel else 0.0
            div = diversity_score(texts) if texts else 0.0
            lat = max(0.0, min(1.0, 200.0 / max(1.0, ms)))  # 200ms == 1.0
            prov = (sum(1 for c in cands if c.get("kind") in ("ppr","expansion")) /
                    max(1, len(cands)))  # graph-traced = better provenance
            criteria = {"recall":recall, "diversity":div, "latency":lat, "provenance":prov}
            for p, w in PERSONAS.items():
                persona_score_sums[p][s].append(omni_score(criteria, w))

    print(f"[UC58d] persona-weighted scores (mean over {len(items)} queries):")
    print(f"{'persona':<16} " + " ".join(f"{s:>10}" for s in strategies))
    rankings = {}
    for p, by_strat in persona_score_sums.items():
        means = {s: statistics.fmean(scores) if scores else 0.0
                 for s, scores in by_strat.items()}
        rankings[p] = sorted(means, key=lambda s: -means[s])
        line = f"{p:<16} " + " ".join(f"{means[s]:>10.3f}" for s in strategies)
        print(line)

    print(f"\n[UC58d] Borda-consensus across personas:")
    for strat, pts in borda_consensus(rankings):
        print(f"  {strat:<10} {pts:>4} pts")
    print("[UC58d] OK")

if __name__ == "__main__":
    main()
