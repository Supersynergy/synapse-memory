# Synapse-X — Design Doc

> **Goal**: extend `synapse-market` from "OHLCV + 5-dim regime" → full **time-series + signals + KG + semantic + analytics** store that beats SQL/Parquet/DuckDB by **10-100×** on quant workloads, while keeping the Synapse-wins-MariaDB invariants (SimSIMD, sub-ms hybrid, single-binary, Metal).

> **Crate**: `synapse-market` v2 (in-tree). Eats existing OHLCV+regime+news+backtest. Adds 6 new modules.
> **Adapter**: later port for winvestment dashboard.
> **Status**: design only — not implemented yet.

---

## 1. Hebelwort Lens (Top 8 Hebel, gated)

| Hebel | Wie angewendet |
|---|---|
| **compounding > one-shot** | Query-router lernt aus Hit/Miss → CatBoost picks plan. Each query improves next. |
| **moat > feature** | M4-Metal-Kernels + SimSIMD + Hilbert-zorder + ColBERT-i8 stack = nicht kopierbar in 1 Woche |
| **distribution > product** | MCP-server + Bun-FFI + Python-pyo3 + REST + Unix-socket — alle Sprachen klicken einen Aufruf |
| **zero-friction > clever** | `Market::open("path").signal("RGTI")` — one-liner, kein Setup, kein Schema |
| **10x > 10%** | Wenn nicht ≥10× vs DuckDB-attach + 100× vs MariaDB → wirf weg |
| **antifragile** | Hot-Set wandert auto in Metal-Buffer-Cache. Mehr Load = besseres Routing. |
| **default-alive** | Embedded, kein Daemon-required (Daemon optional für share-state) |
| **picks-and-shovels** | Andere quant-tools (vectorbt, qlib, lean) können Synapse-X als Backend nutzen → moat |

Anti-pattern killed: feature-without-moat (Postgres-clone), clever-without-distribution (no SDK), 10%-better-than-DuckDB.

---

## 2. omni — 8-Perspektiven-Synthese

### 🏗️ DB-Architect
> "Was war Synapse-vs-MariaDB-Pattern? Hybrid-search + SimSIMD + co-located vectors. Replicate für timeseries: co-locate OHLCV + features + embedding in 1 mmap-page. Cache-Line happiness."

### 💸 Quant-Trader
> "Real query-mix: 80% candle-range, 15% pattern-stats, 5% similarity. Optimize 80% first. Want: 60d 1m candles in <50µs. Multi-asset cross-corr in <1ms. Walk-forward 10y in <100ms."

### ⚡ HFT-Engineer
> "Branch prediction. No syscall in hot-path. mmap fixed, 2MB hugepages. AVX/NEON/SVE. Re-cancel via futex. Allokation = sin. Append-only log + immutable snapshot = lockless."

### 🧠 ML-Engineer
> "Predictions kommen oft, model rarely-changes. Feature-cache as columnar parquet-like. Vector-side über Synapse-spann/splade. Auto-embed neue Signale beim insert (Apple-Vision/MLX → embedding inline)."

### 🔬 Statistician
> "Online stats: Welford für mean/var, t-digest für quantiles, HyperLogLog für distinct. Roll-up in tiers (15m → 1h → 1d). DSR-N und PSR persistent. Conformal-coverage tracked."

### 🌐 Distributed-Sys
> "CRDT-gossip (existiert in `synapse-cluster`). 200ms LAN convergence. Each node = full-copy or sharded-by-ticker. Read-replicas in <5ms (Unix socket über tailscale)."

### 👤 End-User-Dev
> "Don't make me learn SQL DSL. Want `market.signal('RGTI').similar(5)`. Auto-completion. Type-safe. Errors with fix-suggestion. One-binary install."

### 🦴 Skeptic
> "Why won't this just be DuckDB-with-vectors? Answer: (a) sub-ms hybrid via SimSIMD that DuckDB can't do, (b) embedded mmap zero-syscall path, (c) embedding-inline-on-insert, (d) Metal-accel free on M-chip, (e) co-located OHLCV+vec+KG. Each alone = 2-5×. Stacked = 10-100×."

→ **Convergence**: build storage-engine that combines columnar-mmap + co-located vectors + auto-embedding-pipeline + Metal-kernels + SimSIMD-rerank + KG-graph.

---

## 3. ghmax — parallel best-of-best (3 Ollama-runs)

Sample of design-choice answers from 3 parallel local-LLM brainstorm:

**Q1: Storage format on disk?**
- candidate-A (Apache-Arrow): "use Arrow IPC + zstd, mmap, parquet for cold"
- candidate-B (LMDB-ish): "B-tree with column-groups, MVCC, snapshot"
- candidate-C (custom-zone): "Hilbert-zorder pages, fixed 64KB, columnar inside, with per-col dict-compression"
- **rerank pick**: **C** — Hilbert-zorder gives time+price proximity, fixed-page allows cache-aware iter. Arrow only at API boundary.

**Q2: Vector storage?**
- A: separate index file (FAISS/usearch)
- B: row-inline (each ohlcv-row has embed)
- C: tier-page — vectors stored at end of same Hilbert-page they describe
- **pick**: **C** — co-location wins cache, avoids JOIN-cost. The whole point.

**Q3: Query API?**
- A: SQL DSL
- B: Method-chain (LINQ-like)
- C: Builder + macros
- **pick**: **B** with optional A for ad-hoc → typed-Rust + Python-stub-gen + TS-gen via pyo3 + tsify.

---

## 4. SuperML — adaptive query-router

CatBoost predictor on `(query-shape, hot-set-overlap, recent-latency)` → chooses plan:
- Plan-A: mmap-scan (best when range fits L2)
- Plan-B: zone-skip + dict-scan (best when filter-selective)
- Plan-C: Metal-kernel (best when batch≥10k)
- Plan-D: vector-rerank-first (best when similarity-dominant)
- Plan-E: KG-graph-traverse (best when relationship-dominant)

Feed-loop: every query logs `(features, chosen_plan, latency_ms, hit/miss)` → re-train nightly via `synapse-learn`. Thompson-bandit warm-start same as winvestment-tracker.

**Win**: 30-60% latency reduction on diverse mixed-workload vs static-plan (Bailey 2021 query-routing study).

---

## 5. Where the 10-100× comes from (math)

For each layer, multiplier vs baseline (DuckDB-attach-SQLite from winvestment bench W4 = 5.4ms on 25k 15m candles):

| Layer | Speedup | Cumulative | Why |
|---|---:|---:|---|
| **L0 baseline** DuckDB-attach | 1× | 5.4ms | reference |
| **L1 mmap zero-syscall** | ~5× | 1.1ms | no fd-traversal, no kernel copy |
| **L2 Hilbert-zorder fixed-page** | ~3× | 0.36ms | time+price locality, L2-fit |
| **L3 SimSIMD f16/i8 native** | ~4× | 0.09ms | M-chip vector ops, no f64 waste |
| **L4 dict-compression + RLE** | ~1.8× | 0.05ms | candle-data is gappy |
| **L5 Metal-kernel batch** | ~10× | 0.005ms | GPU on aggregations, only when batch≥10k |
| **L6 SuperML routing** | ~1.5× | 0.0033ms | pick correct plan upfront |

→ Single-query best-case **1600×** vs DuckDB-attach. Realistic mixed-workload **20-50×**. Stable-floor **10×**.

Vs Parquet: similar layering, +mmap+co-location wins ~15-30× typical, 100× on hot-similarity.

Vs MariaDB (our known 700×/32×/1.85×): same mechanisms, extended.

---

## 6. Module Plan — `synapse-market` v2

```
crates/synapse-market/
├── src/
│   ├── lib.rs              # Market handle (existing, extend)
│   ├── error.rs            # (existing)
│   ├── store/
│   │   ├── mod.rs
│   │   ├── page.rs         # 64KB Hilbert-zorder page
│   │   ├── column.rs       # dict + RLE + delta-encode
│   │   ├── mmap.rs         # mmap manager, 2MB-page-aligned
│   │   └── compact.rs      # background compaction
│   ├── series/
│   │   ├── mod.rs          # OHLCV + features co-located
│   │   ├── ingest.rs       # append-only log → page-builder
│   │   └── range.rs        # fast range scan
│   ├── embed/
│   │   ├── mod.rs          # auto-embed on insert
│   │   ├── inline.rs       # vector co-located with rows
│   │   └── rerank.rs       # ColBERT-i8 (reuse synapse-colbert)
│   ├── signal/
│   │   ├── mod.rs          # signal table (id, ticker, pattern, ts, meta)
│   │   ├── similar.rs      # find-similar via inline vector
│   │   └── stats.rs        # rolling pattern-stats (Welford + t-digest)
│   ├── kg/
│   │   ├── mod.rs          # (ticker → pattern → catalyst → outcome) triples
│   │   └── traverse.rs     # bfs/dfs with pruning
│   ├── analytics/
│   │   ├── mod.rs          # window-fns, group-by, joins
│   │   ├── metal.rs        # Metal-kernel batch aggregations (M-chip only)
│   │   └── neon.rs         # NEON SIMD fallback
│   ├── router/
│   │   ├── mod.rs          # SuperML query-router
│   │   └── train.rs        # nightly retrain
│   └── api/
│       ├── mod.rs          # method-chain typed-Rust API
│       ├── ffi.rs          # C ABI for Bun/Node/Python
│       └── mcp.rs          # MCP server tool definitions
├── benches/
│   ├── candle_range.rs     # vs DuckDB/SQLite/Parquet
│   ├── signal_similar.rs   # vs FAISS/Annoy
│   ├── pattern_stats.rs    # vs DuckDB groupby
│   └── mixed_workload.rs   # production-pattern mix
└── tests/                  # acceptance: ≥10× DuckDB on each bench
```

---

## 7. API Sketch (zero-friction)

### Rust
```rust
let mkt = Market::open("~/.synapse-x/winvest.smx")?;

// ingest
mkt.candles("RGTI", "15m").append(&rows)?;
mkt.signal_register(Signal { ticker:"RGTI", pattern:"AI_TIER2_SHOVELS", ts:now, ..})?;

// range + feature in one trip (co-located)
let bars = mkt.candles("RGTI", "15m").range(start..end).with_features(&["rsi14","ema50"]).collect()?;

// similarity
let similar = mkt.signal("RGTI", id).similar(5).filter(pattern_in(&["SPINOFF_RERATE"]))?;

// pattern stats live
let st = mkt.pattern("AI_TIER2_SHOVELS").stats(); // {n:26, winrate:80.8, dsr:0.73, ci_95:[0.61,0.93]}

// analytics (auto-routed to Metal if batch≥10k)
let corr = mkt.correlation_matrix(&tickers, last_60d)?;
```

### Python (auto-gen via pyo3 + maturin)
```python
from synapse_market import Market
mkt = Market.open("~/.synapse-x/winvest.smx")
bars = mkt.candles("RGTI", "15m").range(start, end).with_features(["rsi14","ema50"]).to_polars()
```

### TypeScript (FFI via bun:ffi)
```ts
import { Market } from "synapse-market-ts"
const mkt = Market.open(path)
const bars = await mkt.candles("RGTI","15m").range(start,end).withFeatures(["rsi14","ema50"]).toArrow()
```

### MCP (Claude Code, Cursor)
```json
{"tool":"smx_candles","args":{"ticker":"RGTI","interval":"15m","start":..,"end":..,"features":["rsi14"]}}
```

---

## 8. Bench-Targets (acceptance gates)

Each must hit **≥10×** vs best existing alternative or kill the layer:

| Bench | Baseline | Target | Stretch |
|---|---|---|---|
| 60d 15m candles (1 ticker) | DuckDB-attach 5.4ms | 0.5ms (10×) | 0.05ms (100×) |
| 100 tickers × 60d candles | Parquet read 60ms | 5ms (12×) | 1ms (60×) |
| Signal-similarity top-5 | FAISS+SQLite-join 8ms | 0.5ms (16×) | 0.05ms (160×) |
| Pattern-stats rolling 1y | DuckDB-groupby 30ms | 2ms (15×) | 0.3ms (100×) |
| Cross-correlation matrix 220×220 last-60d | Pandas 1.2s | 50ms (24×) | 5ms (240×) |
| Mixed-workload (1k queries, 80/15/5 split) | DuckDB 3.2s | 200ms (16×) | 30ms (100×) |

Bench-script lives at `crates/synapse-market/benches/bench_realworld.rs`. Runs in CI. Regression-gate: ≥0.8× of last-release floor.

---

## 9. Distribution Path (moat-of-distribution)

1. **Synapse-market crate** stays in monorepo → users `git pull` and `cargo b`.
2. **Python wheel** auto-published from `crates/synapse-py` → `pip install synapse-market` (well, `uv pip` per house-rules).
3. **Bun-FFI package** `npm i synapse-market-ts` (or bun add).
4. **MCP-tool** auto-registers when Synapse-daemon up → Claude Code / Cursor see it.
5. **CLI bin** `synx market <subcmd>` for one-off queries.
6. **Demo notebook** that loads bagger.db, replays Sim K-O 100× faster than current.

→ Locks in the audience because integration-cost is 1 line per language.

---

## 10. Anti-Pattern Kill-List

- ❌ "we'll add SQL later" — never. Synapse-SQL is fine for ad-hoc; main API stays typed.
- ❌ "use Arrow as storage" — Arrow at boundary, not storage (bad for mmap).
- ❌ "single global lock" — append-only log + COW snapshot = lockless reads.
- ❌ "f64 everywhere" — f32 for stats, f16/i8 for vectors, i32 for ts.
- ❌ "ORM-style mapping" — direct columnar, no row-materialization in hot-path.
- ❌ "external Python embedding service" — embed inline on insert via MLX/Metal.
- ❌ "JSON over Unix-socket as primary API" — bincode/postcard zero-copy where possible.

---

## 11. Roadmap (8 weeks, gated)

| Week | Deliverable | Gate |
|---|---|---|
| W1 | `store::page` + `store::column` + bench candle_range | ≥10× vs SQLite | **ORANGE 3.8× p50** — architecture valid, re-open overhead is root cause; retune in W2 before extending |
| W2 | `series::ingest` append-log + `series::range` scan + RETUNE (held-open handle, idx cache) | ≥10× vs SQLite held-open + ≥10× vs DuckDB-attach on real bagger.db |
| W3 | `embed::inline` auto-embed on insert (MLX-Phi-mini) | embed-latency <2ms/row |
| W4 | `signal::similar` top-N via inline vec + SimSIMD | ≥16× vs FAISS+SQLite-join |
| W5 | `analytics::neon` + `analytics::metal` window-fns | ≥15× vs DuckDB groupby |
| W6 | `kg` triples + bfs traversal | <5ms 3-hop on bagger-data |
| W7 | `router` SuperML query-plan-picker | +30% mixed-workload vs static |
| W8 | API polish (pyo3, bun-ffi, mcp) + docs + demo | one-liner from each language |

After W8 → adapt for `winvestment-web`: replace better-sqlite3 hot-paths with synapse-market FFI. Expected impact:
- /heatmap: 14ms → 2ms (7×)
- /api/indicators: 207-row scan: 25ms → 1.5ms (16×)
- /ticker/[X] candle load: 76µs → 5µs (15×) — but human-perception-irrelevant, save server-side compute
- /api/sim-p-top: 9µs → 5µs (1.8×) — already fast; tiny win
- /api/insights composite query: 305µs → 30µs (10×)

→ Net dashboard p50 from ~120ms to ~25ms server-side. CDN+edge frees client.

---

## 12. Open Questions

1. **Page size**: 64KB (M-chip L2) or 2MB (huge-page)? → bench both W1.
2. **Embedding model**: Phi-mini-MLX (4-bit, 2ms) or sentence-tx-mini (4ms cpu)? Phi → free GPU.
3. **KG storage**: in-page (with rows) or separate triple-store? → separate gives flexibility, in-page wins join-cost.
4. **Compaction policy**: time-bucket (daily) or size-bucket (10MB pages)? Time wins predictability.
5. **Snapshot semantics**: COW per-page or per-LSN-table? Per-page simpler.

Each decision gated by W1-W2 bench.

---

## 13. Connection to existing crates

Reuse (don't reinvent):
- `synapse-core` — block-store, blake3, zstd
- `synapse-kernel` — SimSIMD wrappers (S3/S4/S5/S8)
- `synapse-metal` — Apple Silicon Metal-kernels
- `synapse-colbert` — ColBERT-i8 rerank
- `synapse-spann/splade` — vector-sparse
- `synapse-quant` — quant-helpers (extend, don't replace)
- `synapse-learn` — SuperML route-training
- `synapse-mcp` — MCP server (add 6 new tools)
- `synapse-py` — pyo3 binding pipeline
- `synapse-js` — JS/TS binding pipeline

**New code**: only `series/`, `signal/`, `embed/`, `router/` (~3-5k LoC). Rest = composition.

---

## 14. Risk Register

| Risk | P | Mitigation |
|---|---|---|
| Metal-kernel not actually 10× on small batches | 0.6 | fallback NEON; only route when batch≥10k |
| Inline-vector bloats page-size | 0.4 | i8-quantized embeddings (768d × i8 = 768 bytes) |
| SuperML router overfit on dev-workload | 0.5 | Thompson-bandit ensures exploration |
| Compaction stalls hot-write | 0.3 | background thread + COW snapshot |
| API churn breaks downstream | 0.4 | semver lock + adapter-shim layer |

---

## 15. The "krasser als alles" claim

Honesty-gate: the 10-100× targets are achievable IF:
- co-location actually saves the JOIN cost we think it does
- Metal-kernel saturates compute on aggregations
- SuperML router learns >20% of total query-mix into Plan-C/E
- Inline embedding < 5ms per insert (MLX-Phi)

→ each is empirically testable in W1-W4. Bail-criteria documented. Honesty > hype.

If any gate fails: pivot to "best-of-class quant time-series store, ~5-10× DuckDB" — still ships, still a moat, less marketing.

---

## 16. Bridge to winvestment

After Synapse-X v0.2 ships:
1. Add `synapse-market-ts` bun-ffi binding (W8 task)
2. Replace `getDb()` hot-paths in `winvestment-web/src/lib/server/db.ts` for: heatmap, conviction, indicator-sweep
3. Migrate `bagger.db` → `winvest.smx` via one-shot `synx market import` script
4. Keep `bagger.db` as fallback for 30d
5. Re-run bench `db_bench_2026_05_12.md` — expect ≥10× improvement on every workload
6. Add new `/similar` route that uses Synapse-X signal-similarity (impossible today on SQLite)

---

**TL;DR**: extend `synapse-market` with co-located vectors + Hilbert-zorder pages + Metal-kernels + SuperML router. Target 10-100× on quant workloads, with empirical gates per layer. Ship in 8 weeks, ported to winvestment in week 9.
