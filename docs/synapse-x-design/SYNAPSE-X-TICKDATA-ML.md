# Synapse-X — Tick-Data & ML-Co-Location (Addendum)

> Extends `SYNAPSE-X-DESIGN.md`. Goal: **100×+ advantage** on tick-data + ML workloads vs Parquet / kdb+ / QuestDB / DuckDB / SQL.

## Hebelwort Lens — neue Axe applied

| Hebel | Wie auf Tick+ML angewendet |
|---|---|
| **co-location > join** | Ticks + features + embedding + ML-state in SAME page → kein read-amplification |
| **write-time materialization** | Features beim Tick-Insert berechnet, nicht beim Query → 1× cost statt N× |
| **streaming > batch** | River-style online-learners statt nightly-rerun → Modell ist nie stale |
| **antifragile** | mehr Ticks = bessere Online-Stats = besseres Routing |
| **picks-and-shovels** | jeder Quant-Strat baut auf SMX → Lock-in via convenience |
| **zero-copy** | mmap → MLX-buffer direkt, kein numpy→torch→mlx roundtrip |
| **compounding** | Yesterday's online-model warm-starts heute → keine cold-start |
| **moat** | kdb+ kostet $200k/y. Wir delivern Edge subset gratis + ML inline. |

---

## 1. Tick-Data Structure — die radikale Idee

### Problem mit Status Quo
- **Parquet**: row-group-batched, kein random-access auf tick-level, schlecht für streaming-insert
- **kdb+**: 200k$/y, proprietär, q-DSL learning curve
- **QuestDB**: ILP-protocol gut, aber kein co-located vec/ml
- **InfluxDB**: tags+fields rigid, kein columnar-friendly
- **SQLite**: 1 row-per-row → tick-data bloat
- **TimescaleDB**: Postgres-extension, OK but kein zero-copy ML

### Synapse-X Tick-Layout

**Tier-Cake** mit 4 Stufen + Hilbert-zorder über (time, price-bucket):

```
T0  WAL (memory ring buffer)          ← hot writes, 8MB ring
T1  L1 page-cache (mmap fixed-pages)  ← 64KB pages, 5-10 sec ticks each
T2  L2 compacted-segments (sealed)    ← 32MB segments, hour-buckets, dict+RLE
T3  L3 cold parquet+zstd-dict-19      ← day-buckets, S3-archivable
```

**Inside each page (T1)**:

```
[ HEADER 64B ]
  magic u32 | flags u32 | n_rows u16 | min_ts i64 | max_ts i64
  min_px f32 | max_px f32 | hilbert_curve_id u8 | blake3_8 u64
[ COLUMNS (delta+frame-of-reference) ]
  ts_delta_i32[n]      → 4n bytes (first ts in header)
  side_bits[n/8]       → 1 bit per tick: buy/sell
  px_delta_i16[n]      → 2n (delta from min_px, scale by exchange-tick-size)
  qty_varint[n]        → 1-5 bytes each, p50≈2
  exchange_id u8[n]    → 1 byte (dict for exchange names)
  trade_cond u8[n]     → 1 byte (NYSE conditions table)
[ FEATURES SLOT — co-located, written by feature-engine ]
  ret_1s f16[n]        → 2n
  vol_z f16[n]         → 2n
  vwap_dev f16[n]      → 2n
  micro_ofi f16[n]     → 2n (order-flow imbalance from book)
  spread_bps u16[n]    → 2n
  ... extensible (declared per-symbol schema)
[ EMBEDDING SLOT — optional ]
  emb_i8[n × 128]      → 128n bytes per quantized embedding
  emb_pq_codes u8[n/k] → product-quantized fallback
[ KG-EDGE SLOT ]
  k links per page → (this_tick → news_id, this_tick → catalyst_id)
[ STATS SLOT ]
  rolling Welford(mean,var) + tdigest tails — 256B per metric
```

**Size budget**: ~50k ticks/page realistic (vs 10k for full-cols). Compression ratio vs raw float64-tick: **8-12×**.

### Why Hilbert-Zorder
- Pure ts-order = great range-scan but lousy if filter is `price between X,Y AND ts ...`
- Pure price-order = vice versa
- Hilbert-curve over (ts, log(price)) = both range-types hit O(√N) pages instead of O(N)
- Critical for tick-replay AND price-range analytics

### Why f16 features
- Apple-MLX prefers f16/bf16 native — zero-copy to GPU
- ML rarely needs f64 precision on features
- 2× smaller than f32, 4× smaller than f64

### Why bit-packed side
- 50k ticks × 1 byte = 50KB. Bit-packed = 6.25KB. CPU branchless extract via PEXT.

---

## 2. Order-Book (L2) Storage

Even more aggressive: book-state event-deltas + checkpoints.

```
[ CHECKPOINT every 1000 events ]
  full 50-level book snapshot (bid+ask × 50 × (px, qty)) ≈ 1KB compressed
[ DELTA stream until next checkpoint ]
  per event: level u8 | side u8 | px_delta_i16 | qty_delta_i32 | op_code u8
  ≈ 8-12 bytes/event
```

**Result**: 1M book-events ≈ 12MB. Reconstruction to any t = chase delta-chain from nearest checkpoint, max 1000 deltas ≈ 8µs.

**Query: "BBO at time T"** = 8µs vs Parquet-rebuild ≈ 5-20ms. **600-2500× win**.

---

## 3. ML Co-Located — die 100× kommt nicht aus speed alleine

### Tier-1: Write-time Feature Materialization
Beim Tick-Insert läuft ein deklarativer Feature-DAG:

```rust
mkt.symbol("RGTI").register_features(features![
    "ret_1s"     => roll_return(1.sec),
    "vol_z"      => rolling_zscore(vol, 20.sec),
    "vwap_dev"   => (px - vwap(60.sec)) / vwap(60.sec),
    "micro_ofi"  => (bid_qty - ask_qty) / (bid_qty + ask_qty),
    "spread_bps" => (ask - bid) / mid * 10000.,
    "embed_128"  => mlx_phi_mini_embed(window=64.ticks),
])?;
```

DAG-engine inkrementell — jeder Feature-Wert in O(1) updaten (Welford, EWMA, ring-buffer). Bei Query-time: **alles schon da**, kein recompute. Speedup vs Parquet+pandas-recompute = **50-200×**.

### Tier-2: Inline Online-Learners
Pro Symbol kann ein Online-Modell laufen, dessen State **in der Stats-Slot der Page** lebt:

```rust
mkt.symbol("RGTI").online_model("dir_prob", FfmFtrl::new(features![...], target!(ret_1s.sign())));
// jeder neue Tick → 1 SGD-step in 200ns inline
```

Modelle die unterstützt werden (alle in `synapse-learn` rust-port von River):
- **FTRL** (logistic-FFM) — direction-probability
- **PassiveAggressive** — regime-classifier
- **HoeffdingTree** — categorical-state
- **OnlineGBM** — boosting (light-online variant)
- **EWRLS** — exponential-weighted recursive least squares for short-horizon return-pred
- **TabPFN-Lite-Rust** — transformer-in-place inference (no-train), für kleine support sets

State persistent in page. Yesterday's weights = today's warm-start.

### Tier-3: Conformal as Column
Jeder Predictor schreibt eine Predicted-CI als zwei zusätzliche f16-Spalten `pred_lower`, `pred_upper`. Split-conformal über rolling-1000-tick window — coverage-Guarantee.

Query-time: trivial filter "show me ticks with pred_upper > current_price + 2σ" = O(scan rows). **Zero ML-roundtrip**.

### Tier-4: Embedding Co-Located + ANN Inline
Per Tick (or per N-tick window): MLX-Phi-mini embed (768d → i8-quantized 128B). 

Embedding-slot in page = page-local mini-index. Query "find ticks similar to current" = SimSIMD scan of last N pages (e.g. 100MB = 5M ticks) in **2-5ms** auf M4 — **kein** ANN-index build, kein FAISS-train.

Globaler Index optional via existing `synapse-spann` für >>1B-tick corpora.

### Tier-5: Causal/Counterfactual (Edge over DuckDB)
EconML-style DoublyRobust streaming estimator → "if I had skipped this news-event, what would PnL be?" Real-time, page-local.

DuckDB / kdb+ liefern das nicht out-of-box.

### Tier-6: MLX-Pipeline Zero-Copy
Page-mmap → MLX buffer alias (same address space). No memcpy. MLX-kernel rechnet auf live-data ohne export. **100× vs pandas-to-torch-to-mlx**.

---

## 4. Realistische ML-Speedups

Bench-Targets (W3-W5):

| Workload | Baseline | Synapse-X | × |
|---|---|---|---|
| 1M-tick feature-extract (10 features) | pandas 8s | inline 0ms (write-time) | **∞** practically |
| Online prediction per-tick | python+sklearn 1.5ms | rust-inline 200ns | **7500×** |
| ANN top-5 over 1M ticks | FAISS-IndexFlatL2 18ms | page-local SimSIMD 1.2ms | **15×** |
| Backtest 100k strategies on tick-stream | vectorbt 12min | rust+page+online-state 38s | **19×** |
| Conformal-CI calibration on rolling 100k | scikit MAPIE 2.4s | inline 14ms | **170×** |
| Causal-effect news-event 30d | EconML-Python 6.8s | streaming 92ms | **74×** |
| Order-book replay to time T | Parquet 12ms | delta-chase 8µs | **1500×** |
| Cross-asset corr-matrix 220×220 last 1d | pandas 920ms | Metal-kernel 6ms | **150×** |

**Stacked dashboard query** (heatmap + conviction + similar + corr) = **80-300×** vs current SvelteKit + better-sqlite + python-rerun stack.

---

## 5. Backend Tests — die ehrliche Pyramide

### L1 — Unit + Property
- `proptest` für jeder Encoder: round-trip f32 candle → page → decode == identity
- `arbitrary` corpus für page-headers
- Bit-pack roundtrip
- Hilbert-zorder injectivity

### L2 — Differential
**Run jeder Query gegen 3 Engines + DIFF:**
```rust
let smx = mkt.candles("RGTI","15m").range(s..e).collect();
let dsl = duckdb_attach(bagger).candles_same();
let sql = sqlite_persist().candles_same();
assert_diff_lte!(smx, dsl, 1e-6);
assert_diff_lte!(smx, sql, 1e-6);
```
Falls je Discrepancy >1e-6 → CI red. CRITICAL: catches off-by-one, dtype-loss, ts-rounding bugs.

### L3 — Fuzzing
- `cargo-fuzz` + libAFL on page-decoder (malformed-input doesn't panic)
- Chaos: random kill mid-write, must recover from WAL
- IO-fault injection (90% writes succeed, 10% return EIO)

### L4 — Snapshot/Replay
- Corpus: real `bagger.db` exported to .smx fixture (10k tickers if available else 220)
- Regression: every new commit replays corpus, latencies stored in `bench-history.db`
- Git-bisect-ready: if perf regresses >10%, bisect tells you the commit

### L5 — Soak (nightly)
- 24h ingestion at 50k tick/sec + concurrent query-mix (80/15/5 split)
- Memory-leak detection via `heaptrack`
- File-descriptor leak check
- Compaction-stall ≤ 5ms p99

### L6 — Cross-Language ABI
- Rust → Python (pyo3) → TS (bun-ffi) → MCP roundtrip with same query
- All 4 must agree byte-for-byte on result

### L7 — Bench-as-Test (acceptance-gate)
- Criterion benches run in CI on M4-runner
- Hard gates: candle-range ≤0.5ms p50, similarity ≤0.5ms, corr-matrix ≤6ms, ingestion ≥50k/sec
- If any gate fails: PR red, can't merge

### L8 — Chaos / Antifragile
- Kill process mid-ingestion, restart → WAL replay, all-or-nothing per-tick
- mmap-coherency: writer + 4 readers, no torn-page
- Cluster: 3 nodes, kill 1, gossip resync within 200ms

---

## 6. ML Test-Discipline (separate from DB tests)

| Test | What |
|---|---|
| **prompt-as-code regression** | Embedding-prompt change → re-run baseline-eval-set, scores must hold ±2% |
| **conformal coverage** | nominal 80% → empirical 78-82% on 1k held-out |
| **drift-monitor** | feature-distribution shift via KL-div alert if >0.2 |
| **two-failure-rule** | model wrong on same setup 2× → auto-quarantine |
| **causal-validity** | parallel-trends test on outcome before treatment |
| **online-vs-batch parity** | online-FTRL after N updates matches batch-train on same N samples ±5% AUC |
| **leakage-audit** | randomize y, re-train, AUC must drop to ~0.5 (no info-leak from features) |
| **shap-stability** | SHAP-values per-feature stable across runs (Jaccard top-10 ≥ 0.7) |
| **adversarial** | randomly perturb 5% features ±1σ → predictions don't flip >10% |
| **MLX-determinism** | same seed → same output across M-chip generations |

CI runs L1-L4 + ML-L1-L5 on each PR. L5-L8 nightly. L7 weekly.

---

## 7. Hebelwort — Picks-and-Shovels Distribution Strategy

Synapse-X als Backend für andere Quant-Tools = compounding moat:

| Tool | Integration |
|---|---|
| **vectorbt** (Py) | adapter exporting `vectorbt.from_synapse(mkt, ticker)` |
| **qlib** | DataHandler subclass `QlibSynapseHandler` |
| **lean** (QuantConnect) | brokerage-data-feed plugin |
| **backtrader** | feed-class |
| **zipline-reloaded** | bundle |
| **NautilusTrader** | data-engine adapter |
| **TradingView Pine** | webhook-pull via Synapse-X REST |
| **R/quantmod** | `synapse_getSymbols()` wrapper |
| **Julia/Lake.jl** | binding |
| **JS-traders** (TV, Bookmap, etc) | WebSocket-stream from Synapse-server |

Each adapter ≤500 LoC. 10 adapters = 5k LoC = 1 week. 

**Distribution-Compounding**: once 3 tools default to Synapse-X data, switching cost is high → moat-by-network-effect.

---

## 8. ML-State Persistence — der echte Killer

Klassisches Setup:
- Daten in Parquet
- Features → pandas
- Model in pickle
- Predictions → CSV
- Recompute alles bei Restart

Synapse-X:
- Daten + Features + Model-state + Conformal-bounds = **alle in der gleichen Page**
- Restart: mmap open → state ist da. **0ms cold-start.**
- "Backtest 2y" = scan pages mit already-computed-features + already-trained-model-state + already-conformal-CI. **Aktuell unmögliche Workload, hier 1-pass.**

Das ist der eigentliche 100× — **wir eliminieren Roundtrips**, nicht nur CPU-cycles.

---

## 9. Sample API (Rust + Py + TS)

### Rust — full ML inline
```rust
let mkt = Market::open("ws.smx")?;

// declarative
mkt.symbol("RGTI")
    .features(features![
        ret_1s, vol_z, vwap_dev, micro_ofi,
        embed_128 => mlx_phi_mini(window: 64)
    ])
    .online_model("dir_prob", FtrlFfm::new())
    .conformal(0.80)
    .ingest_stream(websocket_feed("wss://rgti.feed"))?;

// query (synchronous, page-mmap)
let alerts = mkt.symbol("RGTI")
    .last(10_000_ticks)
    .filter(|t| t.feat("pred_upper") > t.px + 0.5)
    .filter(|t| t.feat("spread_bps") < 5.)
    .similar_to(reference_setup, n: 20)?;
```

### Python
```python
from synapse_market import Market
mkt = Market.open("ws.smx")
df = (mkt.symbol("RGTI")
      .last(10_000)
      .filter("pred_upper > px + 0.5 AND spread_bps < 5")
      .similar_to(ref_setup, n=20)
      .to_polars())  # zero-copy via Arrow C-data interface
```

### TS (Bun)
```ts
import { Market } from "synapse-market-ts"
const mkt = Market.open("ws.smx")
const top = await mkt.symbol("RGTI")
  .last(10_000)
  .filter("pred_upper > px + 0.5")
  .similar(refSetup, 20)
  .toArrow()  // zero-copy ArrayBuffer
```

---

## 10. Risk Register (Tick + ML specific)

| Risk | P | Mitigation |
|---|---|---|
| f16 features lose stat-precision on tail | 0.4 | f32 fallback per-feature decl |
| Online-model drifts under regime-change | 0.7 | drift-monitor + auto-fallback to ensemble |
| Inline embed → ingestion stall | 0.5 | batch-embed every 100 ticks (not per-tick), <1ms overhead |
| Page-local ANN low-recall | 0.4 | global SPANN index optional layer |
| MLX-version churn breaks zero-copy | 0.3 | adapter-shim, version-lock |
| Causal-effect spurious on small-N | 0.7 | confidence-band gate, min-N=200 |

---

## 11. 16-Week Extended Roadmap (Tick+ML)

Original 8-week + 8 weeks for tick/ML/distribution:

| Week | Deliverable |
|---|---|
| W1-W8 | (per main design doc) — store + series + embed + signal + analytics + KG + router + API |
| W9 | Tick-page schema + L2-book delta-storage + bench |
| W10 | Feature-DAG engine + write-time materialization + bench (target 50× vs pandas) |
| W11 | Online-learners (FTRL+EWRLS+HoeffdingTree) + state-in-page |
| W12 | Inline-embedding via MLX + page-local ANN |
| W13 | Conformal-as-column + drift-monitor |
| W14 | Streaming-causal (EconML-rust port subset) |
| W15 | Distribution-adapters: vectorbt, qlib, NautilusTrader |
| W16 | Demo + Whitepaper + open-source-release (Apache-2 + MIT-dual) |

---

## 12. Honesty-Floor

If achievable 100× claim doesn't hit on:
- **tick-feature-materialization** → fallback "5-10× vs pandas, still real win"
- **page-local ANN** → fallback "global SPANN, still 3-5×"
- **online-learners as-good-as batch** → fallback "ensemble-of-online ≈ daily-batch, no recompute"

Ship the working subset honest. **The compounding-of-moats is the real bet, not any single 100×.**

---

## 13. Bridge zu winvestment (W17+)

Once Tick+ML layer stable:
- Add `live_ticks` ingestion via Polygon/Tradier feed in winvestment-web backend
- `/replay/[ticker]` route = bar-by-bar with online-pred-overlay
- `/edge` shows live drift + online-AUC instead of stale stats
- `/optimization` gets real-time refresh (not nightly)
- ML-recommendation per signal = synapse-x inline predictor, not nightly CatBoost

Expected dashboard impact post-W17:
- p50 server-time current 120ms → **8ms**
- New route /tick-replay (impossible today)
- New route /causal-attribution (impossible today)
- Real-time conformal-CI in UI (currently stale)

---

**TL;DR**: tick-data + ML co-located in same mmap-page = **eliminate roundtrips**, not just cycles. 100× kommt aus Architekturschnitt, nicht aus Mikro-Opt. Ship gated, honest, working-subset wins schon.
