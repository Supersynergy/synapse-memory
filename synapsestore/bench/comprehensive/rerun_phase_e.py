#!/usr/bin/env python3
"""Re-run phase E (recall@10) for specified engines and patch existing fast.jsonl."""
import argparse, json, os, sys, tempfile, shutil
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from bench import (
    ADAPTERS, FAST_PROFILE, load_dataset,
    run_phase_recall, run_phase_a,
)

DIR = os.path.dirname(os.path.abspath(__file__))
RESULTS_DIR = os.path.join(DIR, "results")


def rerun_e(engine_name, docs):
    adapter_cls = ADAPTERS[engine_name]
    tmpdir = tempfile.mkdtemp(prefix=f"bench_e_{engine_name}_")
    try:
        adapter = adapter_cls()
        adapter.setup(tmpdir)
        print(f"  [{engine_name}] inserting {len(docs)} docs for recall test...")
        batch_size = 1000
        for i in range(0, len(docs), batch_size):
            adapter.bulk_insert(docs[i:i+batch_size])
        print(f"  [{engine_name}] running phase E...")
        phase_e = run_phase_recall(adapter, docs, n_queries=50)
        phase_e["method"] = "vec_only"
        print(f"  [{engine_name}] recall@10={phase_e['recall_at_10']:.3f}")
        adapter.teardown()
        return phase_e
    except Exception as e:
        print(f"  [{engine_name}] ERROR: {e}")
        return {"recall_at_10": None, "error": str(e)}
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)


def patch_jsonl(engine_name, phase_e):
    path = os.path.join(RESULTS_DIR, f"{engine_name}_fast.jsonl")
    if not os.path.exists(path):
        print(f"  [{engine_name}] No fast.jsonl found, skipping patch")
        return
    with open(path) as f:
        data = json.loads(f.read())
    data["phase_e"] = phase_e
    with open(path, "w") as f:
        f.write(json.dumps(data) + "\n")
    print(f"  [{engine_name}] patched {path}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engines", default="sqlite-vec,lancedb,qdrant,chromadb,synapse",
                        help="Comma-separated engine names")
    args = parser.parse_args()

    n_docs = FAST_PROFILE["n_docs"]
    print(f"[rerun_e] Loading {n_docs} docs...")
    docs = load_dataset(n_docs)
    print(f"[rerun_e] Loaded {len(docs)} docs")

    engines = [e.strip() for e in args.engines.split(",")]
    for engine in engines:
        if engine not in ADAPTERS:
            print(f"[warn] Unknown engine: {engine}")
            continue
        print(f"\n[rerun_e] === {engine} ===")
        phase_e = rerun_e(engine, docs)
        patch_jsonl(engine, phase_e)

    print("\n[rerun_e] Done.")


if __name__ == "__main__":
    main()
