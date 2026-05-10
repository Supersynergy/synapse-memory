#!/usr/bin/env python3
"""UC17 — MS-MARCO recall@K bench (BEIR-style).

Closes the biggest "self-marking" skeptic-gap. Industry-standard recall numbers
on a public dataset rather than self-built N=10k corpus.

Approach:
  1. Load BEIR/MS-MARCO-dev subset (default: 6980 queries, 8.8M passages).
  2. Index passages into a fresh brain.db via `synx put-batch`.
  3. For each query, run `synx hybrid Q --limit K` and `synx ground Q --k K`.
  4. Compute recall@10, recall@100 against qrels.
  5. Report vs published baselines (BM25, BGE-M3, ColBERTv2, etc.).

Defaults to NF-Corpus (smaller, 3.6k queries, 3.6k passages, dev=323 q) for
fast smoke; --large flag escalates to MS-MARCO-dev.

Run:
  python3 eval/usecases/UC17_msmarco_recall.py [--dataset nfcorpus|msmarco]
                                                [--limit 100] [--out results.json]
"""
from __future__ import annotations
import argparse, json, os, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SYNX = os.environ.get("SYNX", "synx")

def ensure_beir(dataset: str) -> Path:
    """Download BEIR dataset if missing. Returns local path."""
    try:
        from beir import util
        from beir.datasets.data_loader import GenericDataLoader
    except ImportError:
        sys.exit("uv pip install beir")
    cache = Path.home() / ".cache" / "beir" / dataset
    cache.parent.mkdir(parents=True, exist_ok=True)
    if not (cache / "corpus.jsonl").exists():
        url = f"https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/{dataset}.zip"
        util.download_and_unzip(url, str(cache.parent))
    return cache

def load_beir(path: Path):
    from beir.datasets.data_loader import GenericDataLoader
    corpus, queries, qrels = GenericDataLoader(str(path)).load(split="test")
    return corpus, queries, qrels

def index_corpus(db: str, corpus: dict, batch_size: int = 100):
    """Bulk-index corpus into synapse brain.db. Returns local-id mapping."""
    id_map = {}  # beir_id → synapse_id
    items = list(corpus.items())
    print(f"[UC17] indexing {len(items)} docs into {db}")
    t0 = time.time()
    for i, (beir_id, doc) in enumerate(items):
        text = (doc.get("title", "") + " " + doc.get("text", "")).strip()
        if not text: continue
        r = subprocess.run([SYNX, "-f", db, "put", "--title", beir_id],
                           input=text, capture_output=True, text=True, timeout=10)
        # synapse put returns the doc id on stdout (best-effort parse)
        out = r.stdout.strip()
        for tok in out.split():
            if tok.isdigit():
                id_map[beir_id] = int(tok); break
    print(f"[UC17] indexed {len(id_map)}/{len(items)} in {time.time()-t0:.1f}s")
    return id_map

def parse_hits(stdout: str) -> list[int]:
    ids = []
    for line in stdout.splitlines():
        parts = line.split("\t")
        if parts and parts[0].isdigit(): ids.append(int(parts[0]))
    return ids

def parse_ground(stdout: str) -> list[int]:
    try:
        d = json.loads(stdout); ids = []
        for k in ("hybrid_seeds", "ppr_ranked", "graph_expansions"):
            for x in d.get(k, []):
                if isinstance(x, dict) and "id" in x: ids.append(int(x["id"]))
                elif isinstance(x, list) and x: ids.append(int(x[0]))
        return ids
    except Exception: return []

def recall_at_k(retrieved: list[int], relevant: list[int], k: int) -> float:
    if not relevant: return 0.0
    return len(set(retrieved[:k]) & set(relevant)) / len(relevant)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dataset", default="nfcorpus", choices=["nfcorpus", "msmarco"])
    ap.add_argument("--limit", type=int, default=100)
    ap.add_argument("--out", default=None)
    ap.add_argument("--db", default="/tmp/synapse_uc17.db")
    ap.add_argument("--strategies", default="hybrid,ground")
    args = ap.parse_args()

    path = ensure_beir(args.dataset)
    corpus, queries, qrels = load_beir(path)
    print(f"[UC17] dataset={args.dataset} corpus={len(corpus)} queries={len(queries)}")

    # init + index
    Path(args.db).unlink(missing_ok=True)
    subprocess.run([SYNX, "-f", args.db, "init"], check=True, capture_output=True)
    id_map = index_corpus(args.db, corpus)

    # eval
    results = {s: {"r@10": [], "r@100": [], "ms": []} for s in args.strategies.split(",")}
    for qid, q in queries.items():
        rel_beir = set(qrels.get(qid, {}).keys())
        rel = [id_map[r] for r in rel_beir if r in id_map]
        if not rel: continue
        for s in args.strategies.split(","):
            t0 = time.time()
            if s == "hybrid":
                r = subprocess.run([SYNX, "-f", args.db, "hybrid", q, "--limit", str(args.limit)],
                                   capture_output=True, text=True, timeout=15)
                ids = parse_hits(r.stdout)
            elif s == "ground":
                r = subprocess.run([SYNX, "-f", args.db, "ground", q, "--k", str(args.limit)],
                                   capture_output=True, text=True, timeout=20)
                ids = parse_ground(r.stdout)
            else: ids = []
            ms = (time.time() - t0) * 1000
            results[s]["r@10"].append(recall_at_k(ids, rel, 10))
            results[s]["r@100"].append(recall_at_k(ids, rel, 100))
            results[s]["ms"].append(ms)

    import statistics
    print(f"\n[UC17] aggregate (mean over {len(queries)} queries):")
    print(f"{'strategy':<10} {'r@10':>8} {'r@100':>8} {'mean_ms':>10}")
    for s, d in results.items():
        r10 = statistics.fmean(d["r@10"]) if d["r@10"] else 0.0
        r100 = statistics.fmean(d["r@100"]) if d["r@100"] else 0.0
        mms = statistics.fmean(d["ms"]) if d["ms"] else 0.0
        print(f"{s:<10} {r10:>8.3f} {r100:>8.3f} {mms:>10.1f}")

    # Published baselines for comparison
    print(f"\n[UC17] published baselines ({args.dataset}):")
    if args.dataset == "nfcorpus":
        print("  BM25 (BEIR paper)        r@10≈0.181 r@100≈0.295")
        print("  ColBERTv2 (Khattab 2022) r@10≈0.276")
        print("  BGE-M3 (BAAI 2024)        r@10≈0.310")
    else:
        print("  BM25 (MS-MARCO-dev)      MRR@10≈0.184")
        print("  ColBERTv2                 MRR@10≈0.397")
        print("  BGE-M3                    MRR@10≈0.405")

    if args.out:
        Path(args.out).write_text(json.dumps({"dataset": args.dataset, "results": results}, indent=2))
        print(f"\n[UC17] saved {args.out}")
    print("[UC17] OK")

if __name__ == "__main__":
    main()
