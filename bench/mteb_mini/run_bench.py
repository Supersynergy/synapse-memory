#!/usr/bin/env python3
"""
MTEB Mini Bench — NFCorpus + SciFact
Models: bge-small-en-v1.5, all-MiniLM-L6-v2
Constraint: only models already in HF cache (no big downloads)
"""
import os, sys, time, json
os.environ["PYTORCH_ENABLE_MPS_FALLBACK"] = "1"
os.environ["CUDA_VISIBLE_DEVICES"] = ""
from pathlib import Path
import mteb

TASKS = ["NFCorpus", "SciFact"]
MODELS = [
    ("BAAI/bge-small-en-v1.5", "bge-small-en-v1.5"),
    ("sentence-transformers/all-MiniLM-L6-v2", "all-MiniLM-L6-v2"),
]

# Published MTEB scores for validation (from MTEB leaderboard, nDCG@10)
PUBLISHED = {
    "bge-small-en-v1.5": {"NFCorpus": 0.327, "SciFact": 0.671},
    "all-MiniLM-L6-v2": {"NFCorpus": 0.321, "SciFact": 0.576},
}

results = {}

for model_id, model_name in MODELS:
    print(f"\n=== {model_name} ===")
    model = mteb.get_model(model_id, device="cpu")

    for task_name in TASKS:
        print(f"  task: {task_name}")
        t0 = time.time()
        try:
            task = mteb.get_task(task_name)
            evaluation = mteb.MTEB(tasks=[task])
            res = evaluation.run(
                model,
                output_folder=f"/tmp/mteb_mini/{model_name}",
                overwrite_results=True,
                verbosity=0,
            )
            elapsed = time.time() - t0
            # Extract nDCG@10 and Recall@10
            ndcg10 = None
            r10 = None
            for r in res:
                scores = r.scores
                if "test" in scores:
                    s = scores["test"][0]
                    ndcg10 = s.get("ndcg_at_10")
                    r10 = s.get("recall_at_10")
                elif scores:
                    first_split = list(scores.values())[0][0]
                    ndcg10 = first_split.get("ndcg_at_10")
                    r10 = first_split.get("recall_at_10")

            pub = PUBLISHED.get(model_name, {}).get(task_name)
            delta = f"{ndcg10 - pub:+.3f}" if (ndcg10 is not None and pub) else "n/a"

            results.setdefault(model_name, {})[task_name] = {
                "ndcg@10": round(ndcg10, 4) if ndcg10 else None,
                "recall@10": round(r10, 4) if r10 else None,
                "published_ndcg@10": pub,
                "delta": delta,
                "elapsed_s": round(elapsed, 1),
            }
            print(f"    nDCG@10={ndcg10:.4f} R@10={r10:.4f} pub={pub} delta={delta} ({elapsed:.0f}s)")
        except Exception as e:
            elapsed = time.time() - t0
            results.setdefault(model_name, {})[task_name] = {"error": str(e), "elapsed_s": round(elapsed, 1)}
            print(f"    ERROR: {e}")

# Dump JSON
out_json = Path("/tmp/mteb_mini_results.json")
out_json.write_text(json.dumps(results, indent=2))
print(f"\nResults saved: {out_json}")
print(json.dumps(results, indent=2))
