#!/usr/bin/env python3
"""UC58c — superml ground-param-predictor.

Trains a CatBoost-Ranker on (query_features, alpha, iters, k, depth) → recall@K
to predict optimal `synx ground` params per query embedding cluster. Hot-path
lookup (~5ms) replaces fixed defaults (alpha=0.5, iters=10, k=20, depth=2).

Pipeline:
  1. For each golden query, sweep param grid {alpha,iters,k,depth} via synx ground.
  2. Score recall@K vs golden relevant_ids.
  3. Build feature matrix: [query_len, vocab_diversity, alpha, iters, k, depth].
  4. Fit CatBoostRanker with group_id=query_id.
  5. Save model to .synapse/ground_params.cbm.
  6. At inference: load model, score every (alpha,iters,k,depth) combo for query, pick max.

Run:
  python3 eval/usecases/UC58c_ground_param_predictor.py train [--db ...] [--golden ...]
  python3 eval/usecases/UC58c_ground_param_predictor.py predict "<query>"
"""
from __future__ import annotations
import argparse, itertools, json, math, os, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SYNX = os.environ.get("SYNX", "synx")
MODEL = ROOT / ".synapse" / "ground_params.cbm"
MODEL.parent.mkdir(parents=True, exist_ok=True)

PARAM_GRID = {
    "alpha":  [0.3, 0.5, 0.7, 0.9],
    "iters":  [5, 10, 20],
    "k":      [10, 20, 30],
    "depth":  [1, 2, 3],
}

def query_features(q: str) -> list[float]:
    toks = q.split()
    n = len(toks)
    diversity = len(set(t.lower() for t in toks)) / max(1, n)
    avg_len = sum(len(t) for t in toks) / max(1, n)
    return [float(n), diversity, avg_len, float(len(q))]

def parse_ground_ids(stdout: str) -> list[int]:
    try:
        d = json.loads(stdout); ids = []
        for k in ("hybrid_seeds", "ppr_ranked", "graph_expansions"):
            for x in d.get(k, []):
                if isinstance(x, dict) and "id" in x: ids.append(int(x["id"]))
                elif isinstance(x, list) and x: ids.append(int(x[0]))
        return ids
    except Exception: return []

def call_ground(db: str, q: str, alpha: float, iters: int, k: int, depth: int) -> tuple[list[int], float]:
    t0 = time.time()
    try:
        r = subprocess.run([SYNX, "-f", db, "ground", q,
                            "--alpha", str(alpha), "--iters", str(iters),
                            "--k", str(k), "--depth", str(depth)],
                           capture_output=True, text=True, timeout=15)
        return parse_ground_ids(r.stdout), (time.time() - t0) * 1000.0
    except Exception:
        return [], (time.time() - t0) * 1000.0

def recall_at_k(retrieved: list[int], relevant: list[int], k: int) -> float:
    if not relevant: return 0.0
    return len(set(retrieved[:k]) & set(relevant)) / len(relevant)

def cmd_train(args):
    try:
        from catboost import CatBoostRanker, Pool
    except ImportError:
        sys.exit("uv pip install catboost")
    golden = Path(args.golden)
    if not golden.is_absolute(): golden = ROOT / args.golden
    if not golden.exists(): sys.exit(f"missing {golden}")
    items = [json.loads(l) for l in golden.read_text().splitlines() if l.strip()]
    items = [it for it in items if it.get("relevant_ids")]
    if len(items) < 5:
        print(f"need ≥5 labeled queries with relevant_ids; have {len(items)}. "
              f"Populate {golden} first.")
        sys.exit(0)
    grid = list(itertools.product(*PARAM_GRID.values()))
    keys = list(PARAM_GRID.keys())
    X, y, groups = [], [], []
    for it in items:
        qid = it["id"]; q = it["query"]; rel = it["relevant_ids"]
        qfeat = query_features(q)
        for combo in grid:
            params = dict(zip(keys, combo))
            ids, _ms = call_ground(args.db, q, **params)
            r = recall_at_k(ids, rel, params["k"])
            X.append(qfeat + [params["alpha"], float(params["iters"]),
                              float(params["k"]), float(params["depth"])])
            y.append(r); groups.append(qid)
    pool = Pool(data=X, label=y, group_id=groups)
    model = CatBoostRanker(iterations=400, depth=6, learning_rate=0.05,
                           loss_function="YetiRank", verbose=50)
    model.fit(pool)
    model.save_model(str(MODEL))
    print(f"saved {MODEL}, n_samples={len(X)}, n_groups={len(set(groups))}")

def cmd_predict(args):
    try:
        from catboost import CatBoostRanker
    except ImportError:
        sys.exit("uv pip install catboost")
    if not MODEL.exists():
        print(json.dumps({"alpha":0.5, "iters":10, "k":20, "depth":2,
                          "fallback":"defaults (no model trained)"}, indent=2))
        return
    m = CatBoostRanker(); m.load_model(str(MODEL))
    qfeat = query_features(args.query)
    grid = list(itertools.product(*PARAM_GRID.values()))
    keys = list(PARAM_GRID.keys())
    rows = [qfeat + list(map(float, c)) for c in grid]
    scores = m.predict(rows)
    best = grid[max(range(len(grid)), key=lambda i: scores[i])]
    out = dict(zip(keys, best))
    out["predicted_score"] = float(max(scores))
    print(json.dumps(out, indent=2))

def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("train"); p.add_argument("--db", default=".synapse/brain.db")
    p.add_argument("--golden", default="eval/golden/grounding.jsonl")
    p.set_defaults(fn=cmd_train)
    p = sub.add_parser("predict"); p.add_argument("query")
    p.set_defaults(fn=cmd_predict)
    a = ap.parse_args(); a.fn(a)

if __name__ == "__main__":
    main()
