"""Stub — 3-way bench harness: faiss · fastembed · synapse on identical BGE embeddings.

Production-ready harness lands with synapse-py v0.3. This stub documents the
shape so the v0.3 ticket has a concrete artefact to fill in.

Planned flow:
    1. Load (or synthesize if model cache missing) BGE-small 384-dim vecs for
       a 100 k-doc corpus (Wikipedia-simple or user-supplied).
    2. Build a FAISS IndexFlatIP (ground truth), a fastembed-in-memory
       store, and a synapse.MultiIndex.
    3. Run 200 held-out queries, measure p50/p95/p99 per engine + recall@10.
    4. Emit a single markdown table into docs/bench_external/v03_results.md.

Current status: not runnable. See `bench/realworld/harness.py` for the
in-repo single-engine flow that already works end-to-end today.
"""

from __future__ import annotations
import sys


def main() -> int:
    print(
        "external 3-way bench (faiss · fastembed · synapse) — stub only.\n"
        "Ships in synapse-py v0.3.\n"
        "For today's numbers, use:\n"
        "  python bench/realworld/aggregator.py\n"
        "or the Rust harness:\n"
        "  cargo run --release -p synapse-core --features 'turbo,simsimd' --example bench_vs_competitors\n",
        file=sys.stderr,
    )
    return 2  # ENOENT-ish: not implemented


if __name__ == "__main__":
    raise SystemExit(main())
