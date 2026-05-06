#!/usr/bin/env python3
"""
LightGBM LambdaMART reranker for Synapse search results.

Features: vec_score, fts_score, recency_days, title_exact_match, path_depth, doc_len_log
Target: click/select signal (binary relevance per query-doc pair)

Usage:
  python lightgbm_rerank.py train --db ~/.synapse/brain.db --out ~/.synapse/rerank.lgb
  python lightgbm_rerank.py score --model ~/.synapse/rerank.lgb --query "search term"
"""
import argparse, json, sqlite3, sys
from pathlib import Path

try:
    import lightgbm as lgb
    import numpy as np
except ImportError:
    print("pip install lightgbm numpy")
    sys.exit(1)

FEATURES = ["vec_score", "fts_score", "recency_days", "title_exact_match", "path_depth", "doc_len_log"]

def build_features(rows: list[dict]) -> np.ndarray:
    out = []
    for r in rows:
        out.append([
            float(r.get("vec_score", 0.0)),
            float(r.get("fts_score", 0.0)),
            float(r.get("recency_days", 365)),
            float(r.get("title_exact_match", 0)),
            float(r.get("path_depth", 3)),
            float(np.log1p(r.get("doc_len", 200))),
        ])
    return np.array(out, dtype=np.float32)

def train(db_path: str, out_path: str):
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    try:
        rows = conn.execute("""
            SELECT query_id, doc_id, vec_score, fts_score, recency_days,
                   title_exact_match, path_depth, doc_len, clicked
            FROM synapse_rerank_log ORDER BY query_id
        """).fetchall()
    except sqlite3.OperationalError:
        print("No synapse_rerank_log table found. Creating synthetic training data.")
        np.random.seed(42)
        n_queries, n_docs = 500, 10
        rows = []
        for q in range(n_queries):
            scores = np.random.dirichlet(np.ones(n_docs))
            for d in range(n_docs):
                rows.append({
                    "query_id": q, "doc_id": d,
                    "vec_score": float(scores[d]),
                    "fts_score": float(np.random.beta(2, 5)),
                    "recency_days": float(np.random.exponential(180)),
                    "title_exact_match": int(np.random.random() < 0.15),
                    "path_depth": int(np.random.randint(1, 6)),
                    "doc_len": int(np.random.lognormal(5, 1)),
                    "clicked": int(d == np.argmax(scores)),
                })
        conn.close()
        rows_dicts = rows
    else:
        rows_dicts = [dict(r) for r in rows]
        conn.close()

    from itertools import groupby
    rows_dicts.sort(key=lambda r: r["query_id"])
    groups = [len(list(g)) for _, g in groupby(rows_dicts, key=lambda r: r["query_id"])]

    X = build_features(rows_dicts)
    y = np.array([r["clicked"] for r in rows_dicts], dtype=np.float32)

    ds = lgb.Dataset(X, label=y, group=groups, feature_name=FEATURES)
    params = {
        "objective": "lambdarank",
        "metric": "ndcg",
        "ndcg_eval_at": [5, 10],
        "learning_rate": 0.05,
        "num_leaves": 31,
        "min_data_in_leaf": 5,
        "verbosity": -1,
    }
    model = lgb.train(params, ds, num_boost_round=200, valid_sets=[ds])
    model.save_model(out_path)
    print(f"Model saved: {out_path}")
    print(f"Feature importance: {dict(zip(FEATURES, model.feature_importance()))}")

def score(model_path: str, candidates: list[dict]) -> list[dict]:
    model = lgb.Booster(model_file=model_path)
    X = build_features(candidates)
    scores = model.predict(X)
    for i, c in enumerate(candidates):
        c["rerank_score"] = float(scores[i])
    return sorted(candidates, key=lambda x: -x["rerank_score"])

def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd")
    t = sub.add_parser("train")
    t.add_argument("--db", default=str(Path.home()/".synapse/brain.db"))
    t.add_argument("--out", default=str(Path.home()/".synapse/rerank.lgb"))
    s = sub.add_parser("score")
    s.add_argument("--model", default=str(Path.home()/".synapse/rerank.lgb"))
    s.add_argument("--candidates", help="JSON array of candidate dicts")
    args = p.parse_args()

    if args.cmd == "train":
        train(args.db, args.out)
    elif args.cmd == "score":
        cands = json.loads(args.candidates or "[]")
        ranked = score(args.model, cands)
        print(json.dumps(ranked, indent=2))
    else:
        p.print_help()

if __name__ == "__main__":
    main()
