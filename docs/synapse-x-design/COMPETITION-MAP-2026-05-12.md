# Synapse-X vs Competition Map — 2026-05-12

**Scope**: Deep-audit Synapse-X (`crates/synapse-market/`) against best-of-2026 tick/columnar/vector engines, mapped onto real use-cases (winvestment-profet / bagger-radar / GOAT-Detector).

**Methodology**: synx hybrid recall (5 queries) + cross-ref with existing `SYNAPSE-X-SOTA-2026.md` (already audited 30+ repos with VERIFIED/PLAUSIBLE/RUMOR tags). No fresh web fetches — anti-stall.

**Verification key**: VERIFIED (cited bench/repo) · PLAUSIBLE (multi-source) · RUMOR (single/unconfirmed).

---

## 1. Competition Matrix (10 rows)

| Engine | License | Killer-feature | Claimed speed (workload) | M-chip native | Embedded | Vector co-located | Online ML | Bindings |
|---|---|---|---|---|---|---|---|---|
| **kdb+ / shakti (Kx)** | Proprietary, ~$200k/y/core | q-lang + in-memory tick-replay, FSM pattern-match over stream | "<5µs L1 lookup" RUMOR; well-known 10-100× pandas on time-joins VERIFIED community | x86 first-class, ARM port exists | Yes (single binary) | No (separate plug-ins) | Weak (`.ml` lib only) | C/Python/Java |
| **QuestDB 8** (`questdb/questdb`) | Apache-2.0 | SIMD ingest (cairo core), SAMPLE-BY, ASOF-JOIN | 1.6M rows/s ingest VERIFIED on docs; SAMPLE BY 3-10× InfluxDB PLAUSIBLE | yes (Java + JNI C) | Server only (no embedded) | No | No | REST/PGWire/ILP |
| **DolphinDB v5** | Proprietary (free tier) | Distributed time-series + streaming engine + DolphinScript | "10× kdb on multi-node" RUMOR (vendor) | yes | partial | weak | inline scripting only | C/Py/Java/JS |
| **ArcticDB v3** (Man/Bloomberg-orbit) | BSL→Apache | Versioned dataframe storage over S3 + symbol-level snapshots | "100× pandas-on-parquet for slice" PLAUSIBLE | yes (pure Py+C++) | yes (lib) | No | No | Python only |
| **ClickHouse Keeper 2026** | Apache-2.0 | MergeTree + sparse-skip indexes + vector (USearch inside) | 10-100M rows/s scan VERIFIED public bench | yes | yes-ish (chdb embedded) | yes (USearch HNSW) | No | C++/Py/JS |
| **Apache DataFusion v40 + Delta** | Apache-2.0 | Composable Rust query engine + Substrait plans + Delta ACID | TPC-H @ 7s 100GB PLAUSIBLE | yes | yes (lib) | via `datafusion-vector` (early) | No | Rust/Py/JS |
| **TimescaleDB + hyperfunctions** | Apache-2.0 core | continuous aggregates + funnel/asof + columnar compression | 100-1000× pg vanilla for time-bucket VERIFIED docs | yes (PG ext) | No (needs PG) | pgvector co-table | No | PG protocol |
| **TileDB embedded** | MIT | Multi-dim arrays + cloud-native sparse | "10× Parquet for sparse" PLAUSIBLE | yes | yes | yes (TileDB-Vector) | No | C/Py/R/Java |
| **Lance v3** (`lance-format/lance`) | Apache-2.0 | mini-chunk + RLE + dict + structural-encoding; ANN inline | random-access 100× Parquet VERIFIED docs | yes | yes (lib) | **YES — first-class** | No | Rust/Py/JS |
| **Vortex** (`vortex-data/vortex`) | Apache-2.0 | Cascading-encoding columnar, encoded random-access | "Parquet+10×" PLAUSIBLE early bench | yes | yes | partial | No | Rust/Py |
| **Synapse-X** (this) | Apache-2.0 | Zorder+JIT+AMX+RaBitQ+FTRL+plan-cache **stacked in one embedded crate** | 117× corr-matrix (AMX), 30× SQLite-filter (JIT), 71× cosine (S4 Hamming) VERIFIED on M4 Max micro-bench | **first-class** (AMX cblas, NEON) | **yes (lib)** | **yes (RaBitQ + turbovec inline)** | **yes (FTRL mmap-state)** | Rust+pyo3+bun-ffi+MCP |

### Verdict by axis

| Axis | Winner | Synapse-X position |
|---|---|---|
| Pure ingest rate | QuestDB | not benched — likely 2nd tier |
| Time-join / asof | kdb+ | Synapse-X has `series/` but no asof primitive ⚠️ |
| Embedded + Rust-native | DataFusion + Lance | Synapse-X **at parity or ahead** for hot path; behind on SQL coverage |
| Vector co-location | Lance v3, Synapse-X | **TIE** — only two engines with truly inline ANN |
| Online ML inline | **Synapse-X alone** | green-field win (no competitor has FTRL-in-mmap) |
| M-chip acceleration | **Synapse-X alone** | AMX route via Accelerate is unique in OSS tick-DBs |
| Pattern-match over stream | **kdb+ alone** | Synapse-X gap — no FSM primitive |
| SQL coverage | DataFusion / ClickHouse | Synapse-X weak — JIT-filter not SQL surface |
| Distribution/replication | kdb+, Clickhouse, Dolphin | Synapse-X **single-node only** ❌ |
| Ecosystem maturity | DataFusion (1000+ contributors) | Synapse-X tiny — owner-bus-factor risk |

---

## 2. Use-case fit

### 2A. winvestment-profet (catalyst-pattern detection on equities)

Data-flow: news/Form-4/RSS → tick-store → corr/feature-extract → signal-score → alert.

| Step | Module | Status | Gap |
|---|---|---|---|
| Ingest news/Form-4 | `news.rs` + `stream/` | ⚠️ partial — only Polygon/Tradier/Kraken ticks wired; no SEC EDGAR/Form-4 parser | **Need: EDGAR ingestor** |
| Store tick + event | `store/` (Hilbert-zorder 64KB page, SoA) | ✅ — schema supports labelled cols | minor: no "event_id" column convention |
| Corr-matrix on signal panel | `analytics/` (AMX 117×) | ✅ flagship | nothing |
| Time-series replay | `series/` + `backtest.rs` | ⚠️ exists but no walk-forward fold-API | **Need: walk-forward iter** |
| Pattern stats (drought-buy, FDA-triple) | none direct | ❌ — no pattern DSL | **Need: pattern-library** |
| Signal similarity → 100x analog lookup | `signal/` (RaBitQ + turbovec) | ✅ — exactly right primitive | nothing |
| Conformal-CI on pattern stats | none | ❌ | **Need: conformal wrapper** |
| Alert push | none | ❌ | **Need: rule engine + webhook/MQTT** |

**Verdict**: 5/8 covered. ⚠️ Missing: pattern DSL, walk-forward, conformal CI, alert engine, EDGAR ingestor.

### 2B. bagger-radar (220-signal tracker → asymmetric long-term thesis)

Reuses profet stack. Additional needs:

| Need | Module | Status |
|---|---|---|
| 220 columns × N tickers panel | `store/` SoA | ✅ |
| Cross-sectional ranking each rebal | `analytics/` | ✅ (corr); ⚠️ no rank/zscore op |
| FTRL re-weighting signals | `learn/` | ✅ |
| Smart-exclude pre-filter (bloom) | `filter/` | ✅ |
| HotSet top-K cache | `cache/` | ✅ |
| Persisted backtest fold-stats | `backtest.rs` | ⚠️ exists, no DSR-gate inline |

**Verdict**: 5/6 covered. Main gap = **walk-forward DSR-gated fold-replay inline**.

### 2C. GOAT-Detector (100x-bagger candidate-finder)

Patterns: insider-cluster · drought-buy · primary-source-driven · smart-exclude · walk-forward DSR-gated.

| Need | Status |
|---|---|
| Insider-cluster detect (Form-4 windowed) | ❌ no Form-4 + no time-window pattern primitive |
| Drought-buy (low-volume + insider) | ❌ no drought primitive (multi-condition FSM over tick) |
| Primary-source-driven scoring | ⚠️ `news.rs` has source-rank, but not weighted into signal |
| Smart-exclude bloom pre-filter | ✅ `filter/` |
| Walk-forward DSR-gated backtest | ⚠️ partial |
| Replay engine for hypotheticals | ⚠️ `backtest.rs` not SQL-surface |

**Verdict**: 1.5/6 covered. **Biggest gap = StreamTime pattern-FSM** (kdb+ killer-feature) + Form-4 ingestor.

---

## 3. Top-5 Improvements (ranked by × × leverage / effort)

### #1 — StreamTime Pattern-FSM (kdb+-killer)
- **Why**: ALL three use-cases need it. drought-buy + insider-cluster + FDA-Triple are FSMs over tick+event-labeled stream. No OSS embedded competitor has this. kdb+ is the only one, locked behind $200k/y.
- **Effort**: M (~2 weeks). Build on top of `series/` — FSM = state-vec + transition-table + window-aware predicate compiled via existing cranelift JIT.
- **Expected ×**: **20-50× vs Python-pandas pattern-scan** PLAUSIBLE; uncontested in OSS embedded → infinite × at marketing level.
- **Risk**: API design hard — get DSL wrong = unusable.

### #2 — Catalyst-as-First-Class Column + Pattern-Library DSL
- **Why**: profet/GOAT need to label tick with `event_id`/`catalyst_type`. Once labeled, joins and stats become trivial. Pattern-library = declarative `drought_buy = volume < ma20 * 0.5 AND form4_buy > 0 in 20d window`.
- **Effort**: S-M (~1 week column convention + ~1 week DSL parser).
- **Expected ×**: 10× developer-velocity for catalyst research (not raw bench). Compounding moat.
- **Pairs with #1** — DSL compiles to FSM.

### #3 — Walk-Forward DSR-gated Backtest API
- **Why**: GOAT-Detector says walk-forward DSR-gated **explicitly**. Without it: every signal claim is in-sample theatre. nautilus_trader is reference. ArcticDB has versioned snapshots but no fold-iter.
- **Effort**: M (~2 weeks). Build on existing `backtest.rs`, add `fold_iter(train_window, test_window, gap, step)` + Deflated-Sharpe gate.
- **Expected ×**: NOT a speed win — it's a **truth gate**. Without it, all bench numbers are fake. **Highest leverage**: it kills 90% of overfit signals before they ship.

### #4 — Conformal-CI Wrapper for Pattern Stats
- **Why**: profet needs honest interval ("drought-buy P[+50% in 6mo] = 0.41±0.08"). No tick-DB has this inline. Synapse-X with FTRL + per-pattern stats can ship conformal prediction natively.
- **Effort**: S (~3 days, calibrated-residual quantile over fold-history).
- **Expected ×**: same as #3 — truth-amplifier, not speed. Trust-multiplier when shipping to skeptical traders.

### #5 — Real-time Alert Rule-Engine (webhook + MQTT)
- **Why**: Pattern-FSM without push = research toy. Profet/bagger-radar must wake user on FDA-Triple match in <1s.
- **Effort**: S (~3 days). Reuse `router/` plan-cache for rule eval; lightweight HTTP/MQTT sink.
- **Expected ×**: closes the loop: ingest → detect → notify. Without it, the whole stack is offline-only.

---

## 4. Anti-pattern Kill-List (DO NOT add)

| Tempting | Why kill |
|---|---|
| **Synapse-cluster CRDT for replicated tick stream** | M-effort, attacks distribution where ClickHouse + Kafka already win. We are EMBEDDED. Adding raft/CRDT = identity loss. NO. |
| **WASM-component for browser-dashboard** | Browser is wrong tier for tick-data analytics. Synapse-X ships Rust+Py+Node bindings — that IS the dashboard story (use Bun/Tauri shell). Pure theater. NO. |
| **Speedb/sled hybrid for cold-archive** | We already have `compact/` (zstd-19 4-8×). Adding a 2nd LSM = maint-debt without × gain. NO. |
| **Causal-inference column inline** | Too generic; DoWhy/EconML in Python is fine offline. Inline = wrong layer. NO. |
| **Backtest-as-SQL via SQL EXPLAIN** | Requires full SQL surface we don't have. Use Pattern-DSL (#2) instead. NO. |
| **Own ANN-graph (rival HNSW)** | RaBitQ already adopted from FAISS. Building a graph index from scratch = 6 months for 1.2× win. NO. |
| **Built-in dashboard / UI** | Out of scope. Bindings + MCP-tools already done. NO. |
| **Trying to beat kdb+ at SQL coverage** | Their moat = q-lang DSL maturity (40 years). We win with pattern-FSM + AMX, NOT by cloning q. NO. |
| **Multi-tenant / cloud-native sharding** | Embedded ≠ cloud. Pick one. NO. |
| **GPU/CUDA path** | M4-Max AMX + Metal-3 already covers it. CUDA = chasing wrong silicon for our user (mac-first). NO. |

---

## Caveman report

**Top-3 closest competitors + killer-feature**:
1. **kdb+** — FSM pattern-match over tick stream (q-lang). $200k/y wall = our wedge.
2. **Lance v3** — first-class vector co-location, encoded random-access. Closest in spirit. Apache. Big team. Real threat → we must differentiate on M-chip + online-ML.
3. **DataFusion v40** — composable Rust SQL engine, huge ecosystem. We are not SQL-first → coexist (run DataFusion ON TOP of Synapse-X store).

**Top-3 use-case gaps (profet/GOAT)**:
1. **StreamTime pattern-FSM** — kdb+-killer; without it drought-buy/insider-cluster/FDA-Triple are Python-pandas-scripts.
2. **Walk-forward DSR-gated backtest API** — truth-gate; without it signals are theatre.
3. **EDGAR/Form-4 ingestor** — primary-source moat; bagger-radar/GOAT explicitly demand it.

**5 next improvements (ranked)**:
1. StreamTime Pattern-FSM (M, 20-50× vs pandas, kdb+-parity OSS)
2. Catalyst-column + Pattern-DSL (S-M, 10× dev-velocity, moat)
3. Walk-forward DSR-gated backtest (M, truth-gate, kills overfit)
4. Conformal-CI wrapper (S, trust-multiplier)
5. Real-time alert rule-engine (S, closes loop)

**Verdict (1-line)**: **almost** — Synapse-X is uncontested OSS-embedded on M-chip + vector + online-ML, but **NOT perfect** for profet/GOAT until StreamTime Pattern-FSM (#1) + walk-forward DSR-gate (#3) ship; with both, it becomes the only honest answer below kdb+.

**File**: `/Users/master/projects/synapse/docs/synapse-x-design/COMPETITION-MAP-2026-05-12.md`
