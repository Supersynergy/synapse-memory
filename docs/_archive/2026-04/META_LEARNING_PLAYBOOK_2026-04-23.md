# Meta-Learning Playbook: Perfecting Synapse-Like DB Projects
*2026-04-23 | Synthesized from PIONEER.md, Synapse bench history, DB release archaeology*

---

## Top-10 Perf-Tricks from DB Release Histories

### [Tier 1 — 10-100× gains, low LOC]

1. **Embedding Hash-Cache (blake3 dedup)** — Every put re-embeds by default. Scraped + commit content has 40-80% dedup rate. `blake3(text) → redb.get(hash)` → skip embed on hit. ~30 LOC, 335ms → 3ms on cache hit. *Pattern: usearch 2.x, LanceDB cache layer, Synapse PIONEER P0.*

2. **WAL + mmap pragma stack** — `PRAGMA journal_mode=WAL; PRAGMA mmap_size=268435456; PRAGMA cache_size=-65536; PRAGMA synchronous=NORMAL`. Single migration, 5-10× write throughput, 3× read throughput. Zero code change. *DuckDB, sqlite-vec, every serious SQLite-backed system.*

3. **Tensor-batched embedding (pad+stack N texts → 1 ONNX forward)** — Sequential embed = SIMD wasted. Batch 32 texts → 1 forward pass via ONNX/Metal → 10× throughput via matmul parallelism. 1000 docs: 5min → 5-15s. *LanceDB batch insert, Qdrant upload redesign v1.2.*

4. **IVF-PQ vector compression** — 384-dim f32 = 1.5KB/doc. IVF-256 + PQ-8 = 48 bytes/doc (31× smaller). Rerank with full vecs at top-50 only. 100M docs on laptop RAM becomes feasible. *faiss foundation, usearch 3.0 introduced PQ, pgvector added HNSW+IVF in v0.6.*

5. **Library-mode crate (zero-IPC)** — Socket/gRPC adds 58µs+ RTT even on localhost. Expose `Db::open + search_hybrid` as a public Rust crate. fn-call latency = sub-µs. Beats sqlite-vec (SQL parser overhead). *LanceDB went library-first in v0.3, DuckDB is always library-first — it's why they win benchmarks.*

6. **RUSTFLAGS target-cpu=native in .cargo/config.toml** — Enables AVX2/AVX-512/NEON depending on host. Free 1.3-2× on SIMD-heavy paths (cosine, dot-product, bloom filters). *Standard in llama.cpp, usearch, qdrant Dockerfile.*

7. **Append-only log + offset index (replace SQLite insert path)** — Writers never block readers. LMDB-style. 10× write throughput. Foundation for real-time stream. *LanceDB fragment model, TigerBeetle journal, Fjall LSM redesign.*

8. **FTS5 trigram tokenizer + BM25 prefilter** — Vector-only recall@10 = 0.38. BM25 prefilter top-200 → vec rerank top-20 = 0.58+. The hybrid is not optional for quality. *DuckDB VSS benchmarks showed this; Qdrant sparse+dense hybrid v1.7.*

9. **Cold-start lazy init** — Don't open DB + load ONNX model at process start. Lazy-init on first query. 700ms cold CLI → <50ms. *Most CLI tools get this wrong until user complaints spike in issues.*

10. **Cross-encoder rerank at L3 (only when margin < ε)** — Only rerank when top-5 similarity scores are clustered (uncertain). Skip on confident results. Adds 0ms most queries, +15 recall@10 when it fires. *ColBERT-lite pattern; Cohere rerank API does this server-side.*

---

## Token-Efficiency Checklist for Synapse Dev Sessions (12 Points)

1. `cargo check` before `cargo build` — type errors in 3s vs 45s full build
2. `cargo nextest run --test-threads=8` — 3× faster than `cargo test`
3. Criterion bench results in commit message body (`Before: 8ms After: 1.2ms`) — future LLM reads git log, no re-run needed
4. `uda ask "<q>"` before any web research — 0ms, 0 API cost, hits prior Synapse dev notes
5. `ctx_batch_execute` for 2+ shell commands — 60× cheaper than Agent spawn
6. `rg "pattern" src/` not Grep tool for multi-file scans in Synapse repo
7. `rtk cargo clippy` / `rtk cargo test` — 80-90% token reduction on output
8. `cargo check --message-format=json | jq '.message.code'` for machine-readable errors
9. Never snapshot full `document.body` or full `cargo build` output — extract specific fields
10. `hyperfetch --extract "term" <url>` (5-12t) not full page fetch (400t+)
11. `difft` for code review diffs — structural diff eliminates noise tokens
12. `sk search "prior similar fix"` — Synapse self-query before starting any optimization work

---

## SuperML Routing Idea for Query Planner

**Verdict: YES — high ROI, ~200 LOC.**

CatBoost classifier on query features:
```
features: [k, dim, filter_selectivity, corpus_size, query_len_tokens, has_filter, is_batch]
targets:  [brute_force, ivf, hnsw, fts5_only, hybrid_rrf, hybrid_l3_rerank]
```

Training data: existing bench suite (`docs/bench_*`) has 50+ (query_type, latency, backend) rows already. Enough for a shallow tree.

**Implementation sketch**:
```rust
// synapse-core/src/planner.rs
pub fn route_query(features: QueryFeatures) -> Backend {
    if features.corpus_size < 1000 { return Backend::BruteForce; }
    if features.has_filter && features.filter_selectivity < 0.01 { return Backend::FTS5Only; }
    if features.k > 50 { return Backend::IVF; }
    Backend::HybridRRF  // default
}
```

Ship rule-based first (above). Replace inner logic with CatBoost `.cbm` model once 500+ bench rows exist. Fallback to rules if model confidence < 0.7 (same pattern as antiban-advise).

---

## "Synapse as its Own Dev-KB" Setup Sketch

```bash
# On every significant commit:
git log -1 --format="%H %s" | \
  xargs -I{} sh -c 'echo "commit: {}\nbench_delta: $(cat /tmp/last_bench_delta.txt 2>/dev/null)" | \
  syn put --tags dev,bench,commit'

# On bench run completion:
echo "Before: $BEFORE After: $AFTER Op: $OP" | syn put --tags bench_result

# Retrieval during dev session:
syn search "why is hybrid search slow" --tags bench_result
# → surfaces prior fix: "2026-04-20 RRF denominator was sqrt(rank) not rank+60"
```

LaunchAgent hook: `~/.claude/hooks/synapse_commit_ingest.sh` — git post-commit → syn put. 5 LOC.

---

## Top-5 Next Learning Sources

1. **LanceDB blog** — `blog.lancedb.com` — posts detail each zero-copy + Arrow optimization with before/after numbers. Better than CHANGELOG alone.
2. **`pgvector` GitHub issues** — community benchmarks drive every design decision. Especially: `ivfflat` vs `hnsw` selectivity threads.
3. **DuckDB internals blog** — `duckdb.org/internals` — vectorized execution + ART index explanations directly applicable to Synapse's FTS5 path.
4. **`fjall` + `redb` source code** — both Rust, both in Synapse's dep tree. Reading their `Cargo.toml` bench targets shows where they focus optimization energy.
5. **BEIR leaderboard** — `beir.ai` — ground truth for recall@10 across 18 datasets. Use as external validator for Synapse's hybrid search quality claims.

---

*Sources: Synapse PIONEER.md, bench history docs/bench_*, PIONEER baseline table, DuckDB/pgvector/LanceDB public release archaeology*
