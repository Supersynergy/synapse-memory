#!/usr/bin/env python3
"""LambdaMART reranker trainer for Synapse learn-to-rank.

Consumes the libsvm file produced by `synapse-learn::QueryLog::export_libsvm`:
    <label> qid:<qid> 1:<bm25> 2:<vec_score> 3:<rank> 4:<score>
    label = clicked (1/0)

Trains a LightGBM LambdaMART ranker, reports NDCG@10 on a query-grouped holdout,
and exports the model. Run with the superml venv (has lightgbm):
    ~/.venvs/superml/bin/python train_rerank_lambdamart.py ~/.synapse/rerank_train.svm

ACTIVATION (real data): build synapse with `--features learn-to-rank` so the daemon
logs query_events + clicks into ~/.synapse/query_log.db, then export via the crate.
Until then `--smoke` proves the pipeline on synthetic graded data.
"""

import argparse
import os
import sys
import tempfile

import numpy as np


def load_grouped(path):
    from sklearn.datasets import load_svmlight_file

    X, y, qid = load_svmlight_file(path, query_id=True)
    X = X.toarray()
    # group sizes in qid order (svmlight is sorted by qid)
    _, idx, counts = np.unique(qid, return_index=True, return_counts=True)
    order = np.argsort(idx)
    return X, y, counts[order]


def make_synthetic(path, n_queries=200, docs=12, seed=0):
    rng = np.random.default_rng(seed)
    lines = []
    for q in range(n_queries):
        # latent relevance driven by features → a learnable signal
        bm25 = rng.random(docs)
        vec = rng.random(docs)
        rank = np.arange(docs)
        score = 0.6 * bm25 + 0.4 * vec + rng.normal(0, 0.05, docs)
        rel = (score > np.quantile(score, 0.7)).astype(int)  # top-30% "clicked"
        for d in range(docs):
            lines.append(
                f"{rel[d]} qid:{q} 1:{bm25[d]:.4f} 2:{vec[d]:.4f} 3:{rank[d]} 4:{score[d]:.4f}"
            )
    open(path, "w").write("\n".join(lines) + "\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("svm", nargs="?", help="libsvm file from export_libsvm")
    ap.add_argument(
        "--smoke",
        action="store_true",
        help="generate synthetic data + train (proves pipeline)",
    )
    ap.add_argument(
        "--out", default=os.path.expanduser("~/.synapse/rerank_lambdamart.txt")
    )
    a = ap.parse_args()
    import lightgbm as lgb

    if a.smoke or not a.svm:
        a.svm = tempfile.mktemp(suffix=".svm")
        make_synthetic(a.svm)
        print(f"[smoke] synthetic libsvm -> {a.svm}")

    X, y, groups = load_grouped(a.svm)
    n = len(groups)
    cut = max(1, int(n * 0.8))
    tr_rows = int(groups[:cut].sum())
    Xtr, ytr, gtr = X[:tr_rows], y[:tr_rows], groups[:cut]
    Xte, yte, gte = X[tr_rows:], y[tr_rows:], groups[cut:]
    print(f"queries={n} train_q={cut} test_q={n - cut} feats={X.shape[1]}")

    ds = lgb.Dataset(Xtr, label=ytr, group=gtr)
    params = dict(
        objective="lambdarank",
        metric="ndcg",
        ndcg_eval_at=[5, 10],
        learning_rate=0.1,
        num_leaves=31,
        min_data_in_leaf=10,
        verbosity=-1,
    )
    evals = {}
    valid_sets, valid_names = [ds], ["train"]
    if len(gte):
        valds = lgb.Dataset(Xte, label=yte, group=gte, reference=ds)
        valid_sets.append(valds)
        valid_names.append("holdout")
    booster = lgb.train(
        params,
        ds,
        num_boost_round=100,
        valid_sets=valid_sets,
        valid_names=valid_names,
        callbacks=[lgb.record_evaluation(evals)],
    )
    if "holdout" in evals:
        print("holdout:", {k: round(v[-1], 4) for k, v in evals["holdout"].items()})
    booster.save_model(a.out)
    print(
        f"model -> {a.out}  (feature_importance={booster.feature_importance().tolist()})"
    )
    print("OK")


if __name__ == "__main__":
    main()
