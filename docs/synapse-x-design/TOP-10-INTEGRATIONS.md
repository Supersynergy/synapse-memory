# TOP-10 Integrations — synapse-market (2026-05-12)

Ranked by ROI = expected_win / effort_cost. Mix of quick-T wins (filters/curves) + medium-leverage cores (RaBitQ, JIT, hftbacktest).

---

## 1. xorf — xor-filter cache layer
- Repo: https://github.com/ayazhafiz/xorf (164★, 2026-02)
- Slot: cache (ingest dedupe, factor universe membership, "have we scored this tick?")
- Effort: **T** — `cargo add xorf`
- ×win: **6×** vs HashSet/Bloom (smaller mem, faster lookup, static)
- **PR spec**: `crates/synapse-market/src/cache/membership.rs` — wrap `xorf::Xor8` for (a) ingest-dedupe symbol+ts keys, (b) factor universe filter pre-eval. Bench vs current HashSet. Goal: -60% mem, -30% lookup ns.

## 2. fastbloom — concurrent ingest dedupe
- Repo: https://github.com/tomtomwombat/fastbloom (348★, 2026-03)
- Slot: cache (high-write streams)
- Effort: **T**
- ×win: **5×** vs std-HashSet under contention; concurrent reads/writes
- **PR spec**: replace any `Arc<Mutex<HashSet>>` in ingest hot-path with `fastbloom::BloomFilter`. Sized for 24h window. Add periodic rotate at midnight UTC.

## 3. fast-hilbert / spacecurve — multidim feature locality
- Repos: https://github.com/becheran/fast-hilbert (60★) + https://github.com/cortesi/spacecurve (527★)
- Slot: store/index (cluster correlated features physically)
- Effort: **T**
- ×win: **5-6×** scan throughput on multidim factor queries; cache-line wins
- **PR spec**: when writing factor frames to columnar store, compute Hilbert-curve key over (symbol_id, sector_id, ts_bucket); sort rows by key before write. Expect 30-50% scan reduction on range queries hitting subspace.

## 4. turbovec — TurboQuant vector index
- Repo: https://github.com/RyanCodrai/turbovec (565★, 2026-05-02)
- Slot: vec (memory tier 2 after MRL-128 / before RaBitQ)
- Effort: **S** (Rust crate with Python bindings; native API)
- ×win: **8×** — newer than RaBitQ, possibly better recall/byte
- **PR spec**: integrate as `synapse_market::recall::turbo` backend. Bench in `rabitq-turboquant-comparison` harness (#17). Pick winner per workload (signal-similarity vs symbol-similarity).

## 5. rabitq-rs — Rust RaBitQ + IVF + MSTG
- Repo: https://github.com/lqhl/rabitq-rs (14★, 2026-02)
- Slot: vec (1-bit terminal tier)
- Effort: **S**
- ×win: **7×** mem reduction vs f16 with bounded error
- **PR spec**: `crates/synapse-market/src/vec/rabitq_backend.rs`. Wire as MRL-128 → RaBitQ-1bit fallback tier. Validate vs official `RaBitQ-Library` (C++) on Sift-1M+factor-vectors. Recall target ≥0.95 at 1-bit.

## 6. cranelift-jit-demo — JIT'd filter/factor predicates
- Repo: https://github.com/bytecodealliance/cranelift-jit-demo (741★)
- Slot: jit (user-defined filters & signal eval)
- Effort: **S** (it IS a demo, copy patterns)
- ×win: **8×** vs tree-walked AST interpreter for repeated eval
- **PR spec**: `crates/synapse-market/src/jit/predicate.rs`. Define a tiny S-expr filter DSL (`(and (gt vol 1e6) (lt spread 0.01))`). JIT-compile via cranelift to `fn(&Row) -> bool`. Cache compiled fns by hash. Bench: 50M rows scan.

## 7. mmap-sync — wait-free hot-data slice
- Repo: https://github.com/cloudflare/mmap-sync (627★, 2026-04)
- Slot: store (read-mostly factor snapshots, symbol metadata)
- Effort: **S**
- ×win: **5×** vs Arc-RwLock-Vec; zero-copy reader
- **PR spec**: replace symbol/feature metadata table with `mmap-sync` writer (1 producer / N readers). Validate under concurrent agent reads. Latency goal: p99 < 100ns reader path.

## 8. vortex — columnar storage core
- Repo: https://github.com/vortex-data/vortex (2926★, 2026-05-12, LFAI)
- Slot: store/analytics (tick-data files, factor frames)
- Effort: **M** (file format change is large but well-supported)
- ×win: **10×** absolute upside (push-down predicates, dict/RLE/BitPack, parquet-killer)
- **PR spec**: introduce `vortex` as the canonical on-disk format for compacted tick & factor data. Keep parquet for interop. Bench scan + predicate-pushdown vs parquet on 1y tick window. Migration: dual-write 30d, then parquet-deprecate.

## 9. hftbacktest — book/match engine blueprint
- Repo: https://github.com/nkaz001/hftbacktest (4050★, 2025-12)
- Slot: book (L2/L3 orderbook, queue position, realistic latency)
- Effort: **M** (vendor the book primitives, not whole project)
- ×win: **10×** vs synthetic-fill backtester
- **PR spec**: import L2/L3 book + queue-pos + latency model into `crates/synapse-market/src/book/`. Wire as the canonical eval harness for execution strategies. Run replay on Binance BTC-USDT 1d → expect realistic slippage curves diverging from naive mid-fill by 5-30 bps.

## 10. art-tree (Adaptive Radix Tree) — price-level index
- Crate: `art-tree` / port of exchange-core ART
- Slot: router (orderbook price-level index, symbol→handle lookup)
- Effort: **S**
- ×win: **6×** vs BTreeMap for prefix-dense integer/string keys
- **PR spec**: replace `BTreeMap<Price, Level>` in book impl with ART. Bench insert/update/range on 10M ops simulated book churn. Targets: -40% lookup ns, -25% mem.

---

## Cross-cutting next steps

- **Bench harness once** (`rabitq-turboquant-comparison`) → drives picks for #4 and #5.
- **AMX direct** (gap #39): wire `accelerate-sys` cblas FFI — 20 LOC, multiplies SimSIMD ceiling. Reference: `apple-silicon-internals` (#37).
- **Welford rolling** (gap #22): in-house ~50 LOC, no external dep. Required by signal pipeline.
- **ONNX inference**: use mature `ort` crate; not in search but standard.

## Order of execution (ship-this-week / 2wk / month)

| Window | Items | Why |
|--------|-------|-----|
| **This week** | #1 xorf, #2 fastbloom, #3 hilbert, gap-#22 Welford | All T-effort, immediate ingest+scan wins |
| **2 weeks**   | #5 rabitq-rs, #4 turbovec (+bench #17), #6 cranelift JIT, #7 mmap-sync, #10 ART | S-effort core upgrades |
| **1 month**   | #8 vortex (storage migration), #9 hftbacktest book primitives, #39 AMX cblas | M-effort, biggest absolute upside |
