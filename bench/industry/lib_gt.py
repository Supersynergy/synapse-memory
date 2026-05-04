"""Shared: load ground truth, compute recall."""
from pathlib import Path
import json, msgpack
import numpy as np

GT_PATH = Path(__file__).parent / "ground_truth.bin"
TEXTS_PATH = Path(__file__).parent / "query_texts.json"
CORPUS_VECS = Path(__file__).parent / "corpus_vecs.npy"
CORPUS_IDS = Path(__file__).parent / "corpus_ids.npy"


def load_gt():
    data = msgpack.unpackb(GT_PATH.read_bytes(), raw=False)
    q_vecs = np.array(data["query_vecs"], dtype=np.float32)
    gt_ids = [list(row) for row in data["gt_ids"]]
    return q_vecs, gt_ids, data


def load_texts():
    d = json.loads(TEXTS_PATH.read_text())
    return d["query_texts"], d["query_doc_ids"]


def load_corpus():
    vecs = np.load(str(CORPUS_VECS))
    ids = np.load(str(CORPUS_IDS))
    return vecs, ids


def recall_at_k(retrieved_ids: list, gt_ids: list, k: int) -> float:
    r = set(retrieved_ids[:k])
    g = set(gt_ids[:k])
    if not g:
        return 1.0
    return len(r & g) / len(g)


def compute_recall(results: list, gt_ids: list, k: int) -> float:
    vals = [recall_at_k(r, g, k) for r, g in zip(results, gt_ids)]
    return float(np.mean(vals))


def pstats(latencies: list) -> dict:
    lat = sorted(latencies)
    n = len(lat)
    mean = float(np.mean(lat))
    return {
        "p50_ms": lat[int(n * 0.50)],
        "p95_ms": lat[int(n * 0.95)],
        "p99_ms": lat[int(n * 0.99)],
        "mean_ms": mean,
        "qps_1c": 1000.0 / mean if mean > 0 else 0,
    }
