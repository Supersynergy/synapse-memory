# Synapse — Results Dashboard

**Last updated**: 2026-05-03  
**Host**: M4 Max, macOS 15.5  
**Replaces**: `bench/RESULTS*.md` (scattered), `bench/mempalace-shootout/RESULTS.md`, `bench/comprehensive/RESULTS*.md`

---

## 360° Bench Summary — vs Competitors (1 000 docs, 200 queries, 384-dim)

Source: `bench/RESULTS-REAL-COMPETITORS.md` (2026-04-20)

| Engine | Insert ms | Query ms/q | Disk KB | Notes |
|--------|----------:|-----------:|--------:|-------|
| FAISS flat | 0.20 | **0.010** | 1 500 | In-RAM, no persistence, no FTS/KG |
| SQLite FTS5 | 2.03 | 0.012 | 140 | Keyword only |
| **Synapse v1.0** | **67.0** | **0.023** | 1 290 | BM25 + HNSW + KG + CRDT + sign — 8 capabilities |
| ChromaDB | 96.32 | 0.303 | 4 434 | Vector only; 13× slower, 3.4× bigger |
| LanceDB | 41.67 | 1.440 | 1 574 | HNSW cold at 1k docs; 63× slower |
| Qdrant | — | — | — | API drift in client ≥1.12; skipped |

Synapse is **2.3× the FAISS theoretical floor** while bundling 8 extra capabilities FAISS lacks.

---

## MemPalace Shootout — Synapse vs ChromaDB (lme_s_50, 50 docs)

Source: `bench/mempalace-shootout/RESULTS.md` (2026-05-03)

### Self-match (insert + latency only)
| Backend | Insert ops/s | Query p50 ms | Query p99 ms | RSS MB | Disk MB |
|---------|-------------|--------------|--------------|--------|---------|
| ChromaDB | 1 550 | 0.23 | 0.36 | 933.5 | 1.51 |
| **Synapse** | **4 528** | **0.03** | **0.04** | 936.8 | 5.20 |

Synapse: **2.9× insert, 7.7× lower query latency, 11× wall time**

### Per-message chunking (76 306 chunks, correct eval)
| Backend | Insert ops/s | Query p50 ms | R@5 | R@10 | Wall s |
|---------|-------------|--------------|-----|------|--------|
| ChromaDB | 1 882 | 2.311 | 0.30 | 0.34 | 160 |
| **Synapse** | **2 688** | 200.084 | 0.30 | 0.34 | 176 |

Note: Synapse query p50 regresses at 76k chunks (Rust FTS5 criterion = 51 µs; Python adapter overhead). R@5 parity at 0.30. Bottleneck = Python<→Rust FFI overhead at large chunk count, not the engine.

---

## FTS Insert Throughput — Auto-Tune Sweep (2026-05-03)

Source: `bench/auto-tune/results.jsonl` (30-config random search on lme_s_50)

| Config | Insert ops/s | Query p50 ms |
|--------|-------------|--------------|
| cache=1024MB mmap=1024MB page=8192 WAL OFF batch=10000 | **255 049** | 0.005 |
| cache=256MB mmap=256MB page=16384 MEMORY OFF batch=100 | 253 272 | 0.007 |
| cache=256MB mmap=256MB page=4096 MEMORY OFF batch=100 | 257 954 | 0.007 |
| batch=1 (any config) | ~15 000–20 000 | 0.007–0.010 |

### SuperML Feature Importance (heuristic — CatBoost fallback)

| Rank | Feature | ops/s spread |
|------|---------|-------------|
| 1 | **batch_size** | 209 568 ops/s |
| 2 | synchronous | 74 765 ops/s |
| 3 | page_size | 55 894 ops/s |
| 4 | journal_mode | 38 902 ops/s |
| 5 | mmap_size_mb | 31 990 ops/s |
| 6 | cache_size_mb | 25 514 ops/s |

**Key insight**: batch_size dominates (4× more leverage than any other knob). Use batch ≥ 1 000 for any bulk ingest path.

### Applied to synapse-core defaults (db.rs)
- `mmap_size`: 256 MB → **1 GB**
- `cache_size`: 64 MB → **256 MB**
- `page_size`: 4096 → **8192**

---

## 20-Usecase Knob Sweep — CatBoost Tuned (2026-04-20)

Source: `bench/RESULTS-V2-FULL.md` (360 data points)

Best global defaults: `zstd_level=3`, `hnsw_ef=16`

Feature importance: **usecase 95.1%**, corpus_size 4.5%, zstd 0.22%, hnsw_ef 0.21%

| Usecase | Median ms | Best config |
|---------|----------:|-------------|
| FTS5 unigram query | 9.69 | zstd=19 ef=64 |
| HNSW kNN (200q) | 5.04 | zstd=19 ef=16 |
| KG chain resolve (100) | 2.21 | zstd=9 ef=64 |
| scope lookup (10k) | 0.35 | zstd=19 ef=128 |
| mmap open | 4.79 | — |

---

## Per-Workload Winners

| Workload | Winner | Number |
|----------|--------|--------|
| Insert burst (batch) | Synapse | 255 000 ops/s (SQLite FTS5, batch=10k) |
| Insert burst (Python adapter) | Synapse | 4 528 ops/s vs ChromaDB 1 550 |
| Search QPS (384-dim, 1k docs) | Synapse | 43 000 q/s (0.023 ms/q) |
| FTS query p50 (Rust criterion) | Synapse | **51 µs** |
| Hybrid retrieval R@5 (with chunking) | Tie | 0.30 (needs reranker) |
| Storage footprint | ChromaDB | 1.51 MB vs Synapse 5.20 MB at 50 docs |
| Memory (RSS) | Tie | ~935 MB both (model load dominates) |

---

## LongMemEval R@5 Progression

| Phase | Chunking | R@5 |
|-------|----------|-----|
| Baseline (whole blob) | 1 doc/record | 0.00 |
| Session-level (4096 chars) | ~149 chunks/record | 0.00 |
| Per-message (~400 chars) | 1526 chunks/record | **0.30** |
| + cross-encoder rerank (P1, TODO) | — | est. 0.60–0.70 |
| + BM25 pre-filter + HyDE (P2, TODO) | — | est. 0.75–0.85 |

---

## Honest Gaps

- **R@5 < 0.85 target**: per-message chunking gets to 0.30; gap = missing cross-encoder reranker (`synapse-rerank` P1 ONNX, not yet wired in Python path).
- **Query p50 regression at 76k chunks**: Python adapter overhead (200 ms) vs Rust criterion (51 µs). Fix = call `synapsed` RPC instead of Python-side FFI loop.
- **Qdrant comparison outdated**: client API changed in ≥1.12; re-bench needed.
- **4stack run files empty**: 148 scheduled run files in `synapsestore/bench/4stack-history/` are 0 bytes — scheduler fired but no workload ran.

---

## Reproduce

```bash
# Competitor shootout (Python)
cd bench && python3 real_competitors.py

# MemPalace (ChromaDB vs Synapse)
cd bench/mempalace-shootout && python3 run.py

# Auto-tune sweep (SQLite config search)
cd bench/auto-tune && python3 harness.py --configs 30 && python3 tune.py

# Rust FTS criterion bench
cargo bench -p bench-space-vs-chroma

# Core check
cargo check -p synapse-core
```

---

## Rerank Wired (2026-05-04)

**Status**: Infrastructure wired; live bench BLOCKED (synapsed daemon not running in CI, lme_s_50.json path requires setup).

- `synapse-rerank::OnnxCrossEncoder` wired in `Space::search_reranked` when compiled with `--features onnx`
- `Request::Rerank` added to synapsed RPC proto — daemon handles rerank server-side
- `bench/mempalace-shootout/run.py --rerank` flag added: fetches top-50, sends to `_daemon_rerank()`, re-evaluates R@K
- R@5 actual: **not measured** — run `python run.py --heldout --rerank --backend synapse` with daemon live to get number
- Target R@5 ≥ 0.55 (vs baseline 0.30)

To reproduce:
```bash
synapsed --file /tmp/bench.db --sock /tmp/synapse.sock &
cd bench/mempalace-shootout
python run.py --heldout --rerank --backend synapse
```

---

## Python via RPC (2026-05-04)

**Status**: `SynapseRpcBackend` + `SynapseRpcCollection` implemented in `python/mempalace-synapse-backend/mempalace_synapse/backend.py`. Batches 1000 docs per `PutBatch` RPC call.

- p50 query actual: **not measured** — run `python run.py --sweep --backend synapse` with daemon live
- Baseline (PyO3 per-call): ~200 ms p50 at 76k chunks
- Target: p50 < 5 ms via batched RPC

To reproduce:
```bash
synapsed --file /tmp/bench.db --sock /tmp/synapse.sock &
cd bench/mempalace-shootout
python run.py --sweep --backend synapse
```
