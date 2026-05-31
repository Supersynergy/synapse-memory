# /// script
# requires-python = ">=3.11"
# dependencies = ["scikit-learn", "numpy", "catboost"]
# ///
"""
train_noise_model.py — SuperML noise/quality classifier for Synapse Context-OS.

Learns which retrieved docs are low-signal noise vs real knowledge, from:
  - context_feedback signal (docs actually used at gate=pass → useful),
  - pattern + structural labels for cold-start (telepathy/status/log/stub → noise;
    known-fact:/decision titles + substantial text → useful).

Trains LogisticRegression (exported, applied natively in Rust) and validates against
CatBoost (held-out AUC). Exports a compact linear model to ~/.synapse/ctxos_noise_model.json
so the Rust hot path applies it with no Python at runtime. Features are byte-for-byte
reproducible in Rust (see crates/synapse-mcp noise_features()).

Run:  uv run tools/ctxos/train_noise_model.py [--brain ~/.synapse/brain.db] [--limit 40000]
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sqlite3
import sys

import numpy as np

FEATURE_ORDER = [
    "log_len",
    "frac_digit",
    "frac_upper",
    "frac_punct",
    "brace_json",
    "log_nlines",
    "avg_line_len",
    "title_marker",
    "uri_log",
    "angle_frac",
    "uniq_ratio",
]


def features(title: str, uri: str, text: str) -> list[float]:
    t = text or ""
    n = len(t)
    title = title or ""
    uri = uri or ""
    digits = sum(c.isdigit() for c in t)
    upper = sum(c.isupper() for c in t)
    punct = sum((not c.isalnum()) and (not c.isspace()) for c in t)
    lines = t.split("\n")
    nlines = len(lines)
    words = t.split()
    nwords = len(words)
    uniq = len({w.lower() for w in words})
    f = {
        "log_len": math.log1p(n),
        "frac_digit": digits / n if n else 0.0,
        "frac_upper": upper / n if n else 0.0,
        "frac_punct": punct / n if n else 0.0,
        "brace_json": 1.0 if ('": ' in t or t.lstrip().startswith("{")) else 0.0,
        "log_nlines": math.log1p(nlines),
        "avg_line_len": (n / nlines) if nlines else 0.0,
        "title_marker": 1.0 if (":" in title or "/" in title) else 0.0,
        "uri_log": 1.0 if uri.endswith(".log") else 0.0,
        "angle_frac": (t.count("<") + t.count(">")) / n if n else 0.0,
        "uniq_ratio": uniq / nwords if nwords else 0.0,
    }
    return [f[k] for k in FEATURE_ORDER]


def is_noise_pattern(title: str, uri: str, text: str) -> bool:
    title, uri, text = title or "", uri or "", text or ""
    if "[telepathy]" in title or "[telepathy]" in text:
        return True
    if "<task-notification>" in text or "tool-use-id" in text:
        return True
    if (
        '"models_loaded"' in text
        or '"desktop_procs"' in text
        or '"cli_sessions"' in text
    ):
        return True
    if uri.endswith(".log") or title.endswith(".log") or "sched_briefing" in title:
        return True
    if text.startswith("Agent [briefing]"):
        return True
    if len(text.strip()) < 40:
        return True
    return False


def is_knowledge(title: str, text: str) -> bool:
    title = title or ""
    return (
        title.startswith("known-fact:")
        or title.startswith("decision/")
        or title.startswith("verified/")
        or (("known-fact" in title or "decision" in title) and len(text) > 200)
    )


def used_ids(con: sqlite3.Connection) -> set[int]:
    ids: set[int] = set()
    for (txt,) in con.execute(
        "SELECT text FROM docs WHERE title LIKE 'ctx-feedback/pass%'"
    ):
        try:
            ids.update(int(i) for i in json.loads(txt).get("used_ids", []))
        except Exception:
            pass
    return ids


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--brain", default=os.path.expanduser("~/.synapse/brain.db"))
    ap.add_argument("--limit", type=int, default=40000)
    ap.add_argument(
        "--out", default=os.path.expanduser("~/.synapse/ctxos_noise_model.json")
    )
    args = ap.parse_args()

    con = sqlite3.connect(args.brain)
    useful_ids = used_ids(con)

    X, y = [], []
    n_noise = n_useful = 0
    rows = con.execute(
        "SELECT id, title, uri, text FROM docs ORDER BY id DESC LIMIT ?",
        (args.limit,),
    )
    for doc_id, title, uri, text in rows:
        title, uri, text = title or "", uri or "", text or ""
        if is_noise_pattern(title, uri, text):
            label = 1
        elif doc_id in useful_ids or is_knowledge(title, text):
            label = 0
        else:
            continue  # unlabeled — skip for training
        X.append(features(title, uri, text))
        y.append(label)
        n_noise += label == 1
        n_useful += label == 0

    if n_noise < 30 or n_useful < 30:
        print(
            f"insufficient labels (noise={n_noise} useful={n_useful}); need >=30 each",
            file=sys.stderr,
        )
        return 1

    X = np.asarray(X, dtype=float)
    y = np.asarray(y, dtype=int)
    print(f"labels: noise={n_noise} useful={n_useful} features={X.shape[1]}")

    from sklearn.linear_model import LogisticRegression
    from sklearn.metrics import roc_auc_score
    from sklearn.model_selection import train_test_split

    Xtr, Xte, ytr, yte = train_test_split(
        X, y, test_size=0.25, random_state=42, stratify=y
    )
    lr = LogisticRegression(max_iter=2000, class_weight="balanced")
    lr.fit(Xtr, ytr)
    lr_auc = roc_auc_score(yte, lr.predict_proba(Xte)[:, 1])
    print(f"LogReg held-out AUC: {lr_auc:.4f}")

    cb = None
    cb_auc = None
    try:
        from catboost import CatBoostClassifier

        cb = CatBoostClassifier(
            iterations=400, depth=6, learning_rate=0.1, verbose=False
        )
        cb.fit(Xtr, ytr)
        cb_auc = float(roc_auc_score(yte, cb.predict_proba(Xte)[:, 1]))
        print(f"CatBoost held-out AUC: {cb_auc:.4f}")
    except Exception as e:  # CatBoost optional
        print(f"CatBoost unavailable: {e}", file=sys.stderr)

    metrics = {
        "logreg_auc": float(lr_auc),
        "catboost_auc": cb_auc,
        "n_noise": int(n_noise),
        "n_useful": int(n_useful),
    }

    # Prefer the tree model (higher AUC); fall back to the exported linear model.
    if cb is not None:
        trees, scale, bias = export_catboost_oblivious(cb)
        model = {
            "version": 2,
            "kind": "catboost_oblivious",
            "feature_order": FEATURE_ORDER,
            "scale": scale,
            "bias": bias,
            "threshold": 0.5,
            "trees": trees,
            "metrics": metrics,
        }
        print(f"exported {len(trees)} oblivious trees")
    else:
        model = {
            "version": 1,
            "kind": "logistic",
            "feature_order": FEATURE_ORDER,
            "weights": {k: float(w) for k, w in zip(FEATURE_ORDER, lr.coef_[0])},
            "bias": float(lr.intercept_[0]),
            "threshold": 0.5,
            "metrics": metrics,
        }

    with open(args.out, "w") as fh:
        json.dump(model, fh)
    print(f"wrote {args.out}")

    # Parity vectors: (features, prob) for the Rust evaluator to match exactly.
    if cb is not None:
        probs = cb.predict_proba(Xte[:8])[:, 1]
        parity = [
            {"features": Xte[i].tolist(), "prob": float(probs[i])}
            for i in range(min(8, len(Xte)))
        ]
        with open(args.out + ".parity.json", "w") as fh:
            json.dump({"feature_order": FEATURE_ORDER, "cases": parity}, fh, indent=2)
        print(f"wrote {args.out}.parity.json ({len(parity)} cases)")
    return 0


def export_catboost_oblivious(cb):
    """Extract CatBoost's oblivious (symmetric) trees into a compact form the Rust
    evaluator applies natively: per tree a list of (feature, border) splits + 2^depth leaves.
    Returns (trees, scale, bias)."""
    import os as _os
    import tempfile

    p = _os.path.join(tempfile.gettempdir(), "ctxos_cb.json")
    cb.save_model(p, format="json")
    with open(p) as fh:
        j = json.load(fh)

    finfo = j.get("features_info", {}).get("float_features", [])
    idx_map = {}
    for ff in finfo:
        fi = ff.get("feature_index")
        flat = ff.get("flat_feature_index", fi)
        if fi is not None:
            idx_map[fi] = flat

    trees = []
    for t in j["oblivious_trees"]:
        splits = []
        for s in t.get("splits", []):
            fi = s.get("float_feature_index")
            flat = idx_map.get(fi, fi)
            splits.append({"feature": int(flat), "border": float(s["border"])})
        trees.append({"splits": splits, "leaves": [float(v) for v in t["leaf_values"]]})

    sb = j.get("scale_and_bias", [1.0, [0.0]])
    scale, bias = 1.0, 0.0
    try:
        scale = float(sb[0][0]) if isinstance(sb[0], list) else float(sb[0])
        b = sb[1]
        bias = float(b[0]) if isinstance(b, (list, tuple)) else float(b)
    except Exception:
        pass
    return trees, scale, bias


if __name__ == "__main__":
    raise SystemExit(main())
