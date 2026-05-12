"""
SuperML vs Thompson-bandit routing benchmark.
Tiers: vec / hybrid / lex / sql
Features: query_len, has_keyword, embed_norm, query_type (0=q,1=cmd,2=code), corpus_size_bucket (0-3)
Label: best_tier from latency × recall trade-off
"""
import numpy as np
import pandas as pd
from sklearn.model_selection import StratifiedKFold, cross_val_score
from sklearn.metrics import f1_score, accuracy_score
from sklearn.preprocessing import LabelEncoder
from sklearn.dummy import DummyClassifier
import warnings
warnings.filterwarnings("ignore")

SEED = 42
np.random.seed(SEED)
N = 1500  # synthetic rows

# ─── Ground-truth latency×recall from bench data ───────────────────────────
# (tier, recall@10, latency_ms_p50) from actual bench results
TIER_PROFILE = {
    "vec":    (0.982, 0.06),   # synapse_usearch_f16
    "hybrid": (0.857, 15.3),   # beir-hybrid (FTS5+vec)
    "lex":    (0.800, 0.1),    # fts5-lex-only baseline
    "sql":    (0.750, 0.5),    # sql exact match (estimated)
}

def best_tier(query_len, has_keyword, embed_norm, query_type, corpus_size_bucket):
    """Deterministic oracle using heuristics grounded in bench data."""
    if query_type == 2:  # code → exact lex
        return "lex"
    if query_type == 1:  # command/keyword → sql or lex
        return "sql" if corpus_size_bucket >= 2 else "lex"
    # natural question
    if has_keyword and query_len < 15:
        return "lex"
    if embed_norm > 0.75 and corpus_size_bucket <= 1:
        return "vec"
    if corpus_size_bucket >= 2:
        return "hybrid"
    return "vec"

# ─── Generate dataset ───────────────────────────────────────────────────────
query_lens = np.random.lognormal(2.5, 0.7, N).clip(3, 200).astype(int)
has_keyword = np.random.binomial(1, 0.35, N)
embed_norm = np.random.beta(4, 2, N)
query_type = np.random.choice([0, 1, 2], N, p=[0.65, 0.20, 0.15])
corpus_size_bucket = np.random.choice([0, 1, 2, 3], N, p=[0.3, 0.3, 0.25, 0.15])

labels = [
    best_tier(query_lens[i], has_keyword[i], embed_norm[i], query_type[i], corpus_size_bucket[i])
    for i in range(N)
]

df = pd.DataFrame({
    "query_len": query_lens,
    "has_keyword": has_keyword,
    "embed_norm": embed_norm,
    "query_type": query_type,
    "corpus_size_bucket": corpus_size_bucket,
    "label": labels,
})

X = df.drop("label", axis=1).values.astype(np.float32)
le = LabelEncoder()
y = le.fit_transform(df["label"].values)
print(f"Dataset: {N} rows, {len(le.classes_)} classes: {le.classes_.tolist()}")
print(f"Label dist: { dict(zip(*np.unique(y, return_counts=True))) }")

cv = StratifiedKFold(n_splits=5, shuffle=True, random_state=SEED)

# ─── Models ─────────────────────────────────────────────────────────────────
results = {}

# 1. Random baseline
rand = DummyClassifier(strategy="stratified", random_state=SEED)
scores = cross_val_score(rand, X, y, cv=cv, scoring="f1_macro")
results["random_baseline"] = {"f1_macro": scores.mean(), "std": scores.std()}

# 2. Static rule (always pick "hybrid" — current synapse-learn fallback)
def static_rule_predict(X):
    # rule: hybrid if corpus>=2, else vec
    out = []
    for row in X:
        cs = int(row[4])
        out.append(le.transform(["hybrid" if cs >= 2 else "vec"])[0])
    return np.array(out)

all_preds_static = []
all_true = []
for train_idx, val_idx in cv.split(X, y):
    preds = static_rule_predict(X[val_idx])
    all_preds_static.extend(preds)
    all_true.extend(y[val_idx])
results["static_rule"] = {
    "f1_macro": f1_score(all_true, all_preds_static, average="macro"),
    "std": 0.0,
    "top1_acc": accuracy_score(all_true, all_preds_static),
}

# 3. Thompson bandit simulation (replays with Beta posteriors, no query features)
class ThompsonBandit:
    def __init__(self, n_classes):
        self.alpha = np.ones(n_classes)
        self.beta = np.ones(n_classes)

    def predict(self, n):
        samples = np.random.beta(self.alpha, self.beta, size=(n, len(self.alpha)))
        return samples.argmax(axis=1)

    def update(self, chosen, correct):
        if correct:
            self.alpha[chosen] += 1
        else:
            self.beta[chosen] += 1

all_preds_tb = []
all_true_tb = []
for train_idx, val_idx in cv.split(X, y):
    tb = ThompsonBandit(len(le.classes_))
    # warm up on train
    for idx in train_idx:
        pred = tb.predict(1)[0]
        tb.update(pred, pred == y[idx])
    # evaluate on val (no features — bandit is feature-blind)
    preds = tb.predict(len(val_idx))
    all_preds_tb.extend(preds)
    all_true_tb.extend(y[val_idx])

results["thompson_bandit"] = {
    "f1_macro": f1_score(all_true_tb, all_preds_tb, average="macro"),
    "std": 0.0,
    "top1_acc": accuracy_score(all_true_tb, all_preds_tb),
}

# 4. LightGBM
try:
    import lightgbm as lgb
    lgb_model = lgb.LGBMClassifier(n_estimators=200, learning_rate=0.05, num_leaves=31,
                                    random_state=SEED, verbose=-1)
    scores = cross_val_score(lgb_model, X, y, cv=cv, scoring="f1_macro")
    all_preds_lgb = []
    all_true_lgb = []
    for train_idx, val_idx in cv.split(X, y):
        lgb_model.fit(X[train_idx], y[train_idx])
        all_preds_lgb.extend(lgb_model.predict(X[val_idx]))
        all_true_lgb.extend(y[val_idx])
    results["lightgbm"] = {
        "f1_macro": scores.mean(), "std": scores.std(),
        "top1_acc": accuracy_score(all_true_lgb, all_preds_lgb),
    }
    print("LightGBM done")
except Exception as e:
    print(f"LightGBM failed: {e}")

# 5. CatBoost
try:
    from catboost import CatBoostClassifier
    cb_model = CatBoostClassifier(iterations=300, learning_rate=0.05,
                                   random_seed=SEED, verbose=0)
    scores = cross_val_score(cb_model, X, y, cv=cv, scoring="f1_macro")
    all_preds_cb = []
    all_true_cb = []
    for train_idx, val_idx in cv.split(X, y):
        cb_model.fit(X[train_idx], y[train_idx])
        preds_raw = cb_model.predict(X[val_idx])
        if hasattr(preds_raw, 'flatten'):
            preds_raw = preds_raw.flatten()
        all_preds_cb.extend([int(p) for p in preds_raw])
        all_true_cb.extend(y[val_idx])
    results["catboost"] = {
        "f1_macro": scores.mean(), "std": scores.std(),
        "top1_acc": accuracy_score(all_true_cb, all_preds_cb),
    }
    print("CatBoost done")
except Exception as e:
    print(f"CatBoost failed: {e}")

# 6. TabPFN
try:
    from tabpfn import TabPFNClassifier
    # TabPFN has 10k sample limit — subsample if needed
    if N > 3000:
        idx = np.random.choice(N, 3000, replace=False)
        Xt, yt = X[idx], y[idx]
    else:
        Xt, yt = X, y
    tabpfn = TabPFNClassifier(device="cpu", n_estimators=4)
    cv_small = StratifiedKFold(n_splits=3, shuffle=True, random_state=SEED)
    scores = cross_val_score(tabpfn, Xt, yt, cv=cv_small, scoring="f1_macro")
    results["tabpfn"] = {"f1_macro": scores.mean(), "std": scores.std()}
    print("TabPFN done")
except Exception as e:
    print(f"TabPFN failed: {e}")

# ─── Print results ──────────────────────────────────────────────────────────
print("\n=== RESULTS ===")
print(f"{'Model':<20} {'F1-macro':>10} {'±':>6} {'Top-1 Acc':>10}")
print("-" * 50)
for model, r in results.items():
    f1 = r["f1_macro"]
    std = r.get("std", 0)
    acc = r.get("top1_acc", float("nan"))
    acc_str = f"{acc:.4f}" if not np.isnan(acc) else "  n/a "
    print(f"{model:<20} {f1:>10.4f} {std:>6.4f} {acc_str:>10}")

# Expected recall and latency per model (using tier profile)
print("\n=== EXPECTED RECALL@10 & LATENCY (p50 ms) per model ===")
tier_to_recall = {le.transform([t])[0]: TIER_PROFILE[t][0] for t in le.classes_}
tier_to_lat    = {le.transform([t])[0]: TIER_PROFILE[t][1] for t in le.classes_}

def expected_metrics(preds, _true=None):
    recalls = [tier_to_recall[int(p)] for p in preds]
    lats    = [tier_to_lat[int(p)]    for p in preds]
    return np.mean(recalls), np.median(lats)

model_preds = {
    "random_baseline": None,
    "static_rule": all_preds_static,
    "thompson_bandit": all_preds_tb,
}
if "lightgbm" in results:
    model_preds["lightgbm"] = [int(p) for p in all_preds_lgb]
if "catboost" in results:
    model_preds["catboost"] = all_preds_cb

print(f"{'Model':<20} {'E[recall@10]':>14} {'E[lat_p50 ms]':>14}")
print("-" * 50)
for model, preds in model_preds.items():
    if preds is None:
        continue
    er, el = expected_metrics(preds, [])
    print(f"{model:<20} {er:>14.4f} {el:>14.4f}")

print("\nDone.")
