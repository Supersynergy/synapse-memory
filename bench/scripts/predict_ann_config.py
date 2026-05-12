#!/usr/bin/env python3
"""
ANN parameter predictor — TabPFN v2 / CatBoost
Usage:
  predict_ann_config.py train
  predict_ann_config.py predict '{"n_docs": 168000, "dim": 384, "target_recall": 0.99}'
  predict_ann_config.py coverage   # check data coverage gaps
"""
from __future__ import annotations

import json
import math
import os
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RESULTS_DIR = REPO / "bench" / "results"
DATA_DIR = REPO / "data"
MODEL_PATH = DATA_DIR / "ann_param_predictor.cbm"
CSV_PATH = DATA_DIR / "ann_bench_aggregated.csv"

# ─── 1. Aggregation ─────────────────────────────────────────────────────────

def parse_engine_name(name: str) -> dict:
    """Extract (engine_type, M, ef_construct, ef_search, quant) from engine string."""
    info: dict = {"M": None, "ef": None, "quant": "none"}
    n = name.lower()
    if "usearch" in n or "qdrant" in n:
        info["engine_type"] = "hnsw"
    elif "lance ivf_hnsw_sq" in n:
        info["engine_type"] = "hnsw"
        info["quant"] = "sq"
    elif "lance ivf_pq" in n:
        info["engine_type"] = "ivf_pq"
        info["quant"] = "pq"
    elif "sqlite" in n:
        info["engine_type"] = "brute"
    elif "ultra" in n:
        info["engine_type"] = "binary"
        info["quant"] = "binary"
    else:
        info["engine_type"] = "unknown"

    # parse M= and ef=
    import re
    m = re.search(r"m=(\d+)", n)
    ef = re.search(r"ef=(\d+)", n)
    if m:
        info["M"] = int(m.group(1))
    if ef:
        info["ef"] = int(ef.group(1))
    return info


def collect_rows() -> list[dict]:
    rows = []
    for json_path in RESULTS_DIR.rglob("*.json"):
        if json_path.parent.name == "raw":
            continue
        try:
            text = json_path.read_text()
            # strip log lines before JSON
            lines = text.splitlines()
            json_start = next((i for i, l in enumerate(lines) if l.strip().startswith(("{", "["))), 0)
            data = json.loads("\n".join(lines[json_start:]))
        except Exception:
            continue

        # normalise: summary.json has "results" key
        if isinstance(data, dict) and "results" in data:
            entries = data["results"]
            meta = {k: v for k, v in data.items() if k != "results"}
        elif isinstance(data, list):
            entries = data
            meta = {}
        else:
            continue

        for e in entries:
            if not isinstance(e, dict):
                continue
            engine = e.get("engine", "")
            recall = e.get("recall_at_10") or e.get("recall_at_10_vs_gt")
            qps = e.get("qps_1c")
            if recall is None or qps is None:
                continue

            info = parse_engine_name(engine)
            # n_docs: from note if capped, else infer from file path label
            n_docs = None
            note = e.get("note", "")
            import re
            m = re.search(r"(\d{4,})\b", note)
            if m:
                n_docs = int(m.group(1))
            # industry bench: 168438
            if n_docs is None and "industry" in str(json_path):
                n_docs = 168438
            if n_docs is None:
                continue

            rows.append({
                "n_docs": n_docs,
                "dim": 384,  # all synapse benches use all-minilm 384-dim
                "M": info["M"] or 16,
                "ef_construct": info["ef"] or 64,
                "ef_search": info["ef"] or 64,
                "quant": info["quant"],
                "engine_type": info["engine_type"],
                "recall_at_10": float(recall),
                "qps": float(qps),
                "source": str(json_path.relative_to(REPO)),
            })
    return rows


# ─── 2. Synthetic augmentation (HNSW scaling laws) ──────────────────────────
# Based on empirical HNSW behaviour: recall grows with M and ef, QPS shrinks.
# Anchor: usearch M=16,ef=64 → recall=0.924, QPS=4853 at n=168k, dim=384

def synthetic_rows() -> list[dict]:
    rows = []
    anchor_recall = 0.924
    anchor_qps = 4853.0
    anchor_n = 168438
    anchor_dim = 384

    for n_docs in [10_000, 50_000, 168438, 500_000, 1_000_000]:
        for dim in [128, 256, 384, 768]:
            for M in [8, 12, 16, 24, 32, 48]:
                for ef in [32, 64, 128, 256, 512]:
                    # recall model: recall ≈ 1 - exp(-k * M * ef)
                    # fitted from anchor: k = -ln(1-0.924)/(16*64)
                    k = -math.log(1 - anchor_recall) / (16 * 64)
                    recall = 1.0 - math.exp(-k * M * ef)
                    recall = min(recall, 0.9999)

                    # QPS model: QPS ∝ 1/(ef * log(n) * dim^0.5) * M^0.3
                    # normalised to anchor
                    qps_scale = (
                        (anchor_ef := 64) / ef
                        * (math.log(anchor_n) / math.log(n_docs))
                        * ((anchor_dim / dim) ** 0.5)
                        * ((M / 16) ** 0.3)
                    )
                    qps = anchor_qps * qps_scale

                    rows.append({
                        "n_docs": n_docs,
                        "dim": dim,
                        "M": M,
                        "ef_construct": ef,
                        "ef_search": ef,
                        "quant": "none",
                        "engine_type": "hnsw",
                        "recall_at_10": round(recall, 5),
                        "qps": round(qps, 2),
                        "source": "synthetic",
                    })
    return rows


# ─── 3. Feature encoding ─────────────────────────────────────────────────────

QUANT_MAP = {"none": 0, "sq": 1, "pq": 2, "binary": 3}
ENGINE_MAP = {"hnsw": 0, "ivf_pq": 1, "brute": 2, "binary": 3, "unknown": 4}

def row_to_features(r: dict) -> list[float]:
    return [
        math.log1p(r["n_docs"]),
        math.log1p(r["dim"]),
        float(r["M"]),
        math.log1p(r["ef_construct"]),
        math.log1p(r["ef_search"]),
        float(QUANT_MAP.get(r["quant"], 0)),
        float(ENGINE_MAP.get(r["engine_type"], 0)),
    ]

FEATURE_NAMES = ["log_n_docs", "log_dim", "M", "log_ef_construct", "log_ef_search", "quant_enc", "engine_enc"]


# ─── 4. Train ────────────────────────────────────────────────────────────────

def train():
    real_rows = collect_rows()
    synth = synthetic_rows()
    all_rows = real_rows + synth

    print(f"Real rows: {len(real_rows)}, Synthetic rows: {len(synth)}, Total: {len(all_rows)}")
    if len(real_rows) < 50:
        print(f"WARNING: Only {len(real_rows)} real bench rows — using synthetic augmentation.")
        print("To improve: run bench with varied (M, ef, n_docs, dim) and add to bench/results/.")

    # Filter rows with valid HNSW params (skip brute/binary for recall predictor)
    model_rows = [r for r in all_rows if r["M"] > 0 and r["ef_search"] > 0]

    X = [row_to_features(r) for r in model_rows]
    y_recall = [r["recall_at_10"] for r in model_rows]
    y_qps = [math.log1p(r["qps"]) for r in model_rows]

    from catboost import CatBoostRegressor, Pool

    # Mark real rows with higher weight
    weights = [10.0 if r["source"] != "synthetic" else 1.0 for r in model_rows]

    def fit(y, name):
        model = CatBoostRegressor(
            iterations=500, depth=6, learning_rate=0.05,
            loss_function="RMSE", verbose=False,
        )
        model.fit(Pool(X, y, weight=weights, feature_names=FEATURE_NAMES))
        return model

    m_recall = fit(y_recall, "recall")
    m_qps = fit(y_qps, "qps")

    # Save models
    DATA_DIR.mkdir(exist_ok=True)
    m_recall.save_model(str(MODEL_PATH.with_suffix(".recall.cbm")))
    m_qps.save_model(str(MODEL_PATH.with_suffix(".qps.cbm")))
    print(f"Saved: {MODEL_PATH.with_suffix('.recall.cbm')}")
    print(f"Saved: {MODEL_PATH.with_suffix('.qps.cbm')}")

    # Save CSV
    import csv
    with open(CSV_PATH, "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=list(model_rows[0].keys()))
        writer.writeheader()
        writer.writerows(model_rows)
    print(f"Saved CSV: {CSV_PATH} ({len(model_rows)} rows)")

    # Quick validation on real rows only
    if real_rows:
        from sklearn.metrics import mean_absolute_error
        real_X = [row_to_features(r) for r in real_rows]
        pred_recall = m_recall.predict(real_X)
        pred_qps = [math.expm1(v) for v in m_qps.predict(real_X)]
        mae_recall = mean_absolute_error([r["recall_at_10"] for r in real_rows], pred_recall)
        mae_qps = mean_absolute_error([r["qps"] for r in real_rows], pred_qps)
        print(f"In-sample MAE (real rows): recall={mae_recall:.4f}, qps={mae_qps:.1f}")
        print("Note: in-sample metrics (too few rows for held-out validation)")


# ─── 5. Predict ──────────────────────────────────────────────────────────────

def predict(request: dict) -> dict:
    n_docs = int(request["n_docs"])
    dim = int(request["dim"])
    target_recall = float(request["target_recall"])

    recall_model_path = MODEL_PATH.with_suffix(".recall.cbm")
    qps_model_path = MODEL_PATH.with_suffix(".qps.cbm")
    if not recall_model_path.exists():
        return {"error": "Model not trained. Run: predict_ann_config.py train"}

    from catboost import CatBoostRegressor
    m_recall = CatBoostRegressor()
    m_recall.load_model(str(recall_model_path))
    m_qps = CatBoostRegressor()
    m_qps.load_model(str(qps_model_path))

    # Grid search over HNSW params
    best = None
    best_qps = -1.0
    candidates = []
    for M in [8, 12, 16, 24, 32, 48]:
        for ef in [32, 64, 128, 256, 512]:
            feat = row_to_features({
                "n_docs": n_docs, "dim": dim, "M": M,
                "ef_construct": ef, "ef_search": ef,
                "quant": "none", "engine_type": "hnsw",
            })
            pred_recall = float(m_recall.predict([feat])[0])
            pred_qps = math.expm1(float(m_qps.predict([feat])[0]))
            if pred_recall >= target_recall and pred_qps > best_qps:
                best_qps = pred_qps
                best = {
                    "M": M, "ef_construct": ef, "ef_search": ef,
                    "quant": "none",
                    "expected_recall": round(pred_recall, 4),
                    "expected_qps": round(pred_qps, 1),
                }

    if best is None:
        # Relax: find closest to target_recall
        for M in [32, 48]:
            for ef in [256, 512]:
                feat = row_to_features({
                    "n_docs": n_docs, "dim": dim, "M": M,
                    "ef_construct": ef, "ef_search": ef,
                    "quant": "none", "engine_type": "hnsw",
                })
                pred_recall = float(m_recall.predict([feat])[0])
                pred_qps = math.expm1(float(m_qps.predict([feat])[0]))
                candidates.append((pred_recall, pred_qps, M, ef))
        candidates.sort(key=lambda x: -x[0])
        r = candidates[0]
        best = {
            "M": r[2], "ef_construct": r[3], "ef_search": r[3],
            "quant": "none",
            "expected_recall": round(r[0], 4),
            "expected_qps": round(r[1], 1),
            "warning": f"Could not reach target_recall={target_recall}. Best achievable: {r[0]:.4f}",
        }

    return best


# ─── 6. Coverage report ──────────────────────────────────────────────────────

def coverage():
    rows = collect_rows()
    print(f"Real bench rows: {len(rows)}")
    if len(rows) < 50:
        needed = 50 - len(rows)
        print(f"INSUFFICIENT: need {needed} more rows for reliable model.")
        print("Suggested configs to bench:")
        for M in [8, 24, 32]:
            for ef in [128, 256]:
                print(f"  usearch M={M} ef={ef} (n=168438, dim=384)")
        for n in [10000, 50000, 500000]:
            print(f"  usearch M=16 ef=64 (n={n}, dim=384)")
    else:
        print("Coverage sufficient for CatBoost training.")
    if rows:
        n_docs_vals = sorted({r["n_docs"] for r in rows})
        M_vals = sorted({r["M"] for r in rows})
        ef_vals = sorted({r["ef_search"] for r in rows})
        print(f"n_docs seen: {n_docs_vals}")
        print(f"M seen: {M_vals}")
        print(f"ef seen: {ef_vals}")


# ─── main ─────────────────────────────────────────────────────────────────────

def main():
    if len(sys.argv) < 2 or sys.argv[1] == "train":
        train()
    elif sys.argv[1] == "predict":
        req = json.loads(sys.argv[2] if len(sys.argv) > 2 else sys.stdin.read())
        result = predict(req)
        print(json.dumps(result, indent=2))
    elif sys.argv[1] == "coverage":
        coverage()
    else:
        print(__doc__)
        sys.exit(1)


if __name__ == "__main__":
    main()
