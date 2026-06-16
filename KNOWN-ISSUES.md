# Known Issues

## LongMemEval R@5 below target

**Severity**: Medium (quality metric)
**Status (2026-06-16, corrected)**: the old "R@5 = 0.30, reranker not yet wired" is **stale and wrong**.
- The cross-encoder reranker (`synapse-rerank`, BGE-reranker-v2-m3 ONNX) **is** wired into the runner (`bench/longmemeval/src/main.rs:516-535`, gated on `--features rerank --rerank-top N`).
- Verified lexical-only baseline (`--no-default-features --rerank-top 0`) = **R@5 0.640** (`docs/LONGMEMEVAL_RESULTS_2026-05-25.md`).
- Vector recall is effectively solved (Hamming-cascade ANN R@10 ≈ 0.994, `crates/synapse-core/src/db.rs:1631`).
**Full-stack run (2026-06-16, measured this session)**: `--features "embed-768,rerank" --embed --rerank-top 20 --limit 50`, Arctic-Embed-M 768d + BGE-reranker-v2-m3:
- Recall@5 **0.640** (32/50) · Fuzzy-R@5 0.660 · latency **3503 ms/q** (vs 0.97 ms lexical).
- **Finding: vector + cross-encoder rerank gave ZERO R@5 gain over lexical-only on this 50-q subset, at ~3600× latency.** The earlier hypothesis that the published 0.640 was "running blind" is **refuted** — 0.640 is the real ceiling for this config; ranking is NOT the bottleneck.
**Real remaining gap**: the limit is upstream — chunking / candidate coverage (Avg docs/Q 47.3; the ~18 misses likely never have the gold chunk in the candidate set), or the eval subset itself. Improving R@5 needs a retrieval-coverage lever (chunking, query expansion/HyDE, larger candidate pool), not reranking. Larger N (full LongMemEval, not 50) also needed before trusting the delta.

---

## synapse-ann / synapse-wal / synapse-seg — stub crates

**Severity**: Low (no functionality blocked)
**Crates**: `synapse-ann`, `synapse-wal`, `synapse-seg`
**Status**: Scaffolded with TODO markers. Scale-100M and crash-safe ingest paths not yet implemented.

---

## Corrected Marketing Claims (Day-13 audit 2026-04-25)

See [CORRECTIVE-ACTION-PLAN-2026-04-25.md](CORRECTIVE-ACTION-PLAN-2026-04-25.md) for full audit log.

| Claim | Was | Corrected to |
|-------|-----|-------------|
| WP speedup | "100× MySQL / 100× WP" | "5-10× WP overall; 30-100× search-only at 50k+ posts" |
| WP search | "187× WP search" | "50× LIKE→FTS5 + 8× FTS5→semantic + 1.2× overhead ≈ 480× compounded at 50k+ posts; vanilla LIKE wins at <1k posts" |
| Recall | "Recall 1.000 always" | "Recall 1.000 dense / ≥0.95 quantized (int8/binary)" |
| Library latency | "5µs reads" | "<50µs at 100k+ docs; daemon mode wins below 100k (WAL+mmap warm cache)" |
| Not shipped | synapse-wasm, synapse-mobile | Removed from claims |
| Customers | "enterprise pilot conversations" | No real customers; target is "paying design-partners"
