# Integration Candidates — synapse-market (2026-05-12)

Source: 10 targeted `gh search repos` queries (ghmax repo-mode broken, fell back to raw gh CLI). Filters: stars >=20-200 depending on space, sorted by stars desc, recent activity preferred.

Slot legend: **store** (mmap/columnar) · **cache** (filter) · **filter** (predicate/JIT) · **jit** (codegen) · **signal** (online ML) · **book** (orderbook/tickdb) · **router** (radix) · **analytics** (columnar/quant) · **vec** (vector index).

Effort: T=trivial (cargo add, ≤1 day) · S=small (1-3 days adapter) · M=medium (1-2 weeks port) · L=large (>2 weeks).

× win = expected leverage if integrated (qualitative bucket).

## Q1 — Columnar / mmap store

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 1 | https://github.com/vortex-data/vortex | 2926 | 2026-05-12 | store/analytics | M | **10×** | LFAI columnar file format, fastest FOSS, push-down predicates, dict/RLE/BitPack codecs. Direct replacement for parquet hot-path. |
| 2 | https://github.com/cloudflare/mmap-sync | 627 | 2026-04-28 | store | S | **5×** | mmap zero-copy, wait-free reader/writer split. Drop-in for hot read-mostly slices (tickers metadata, feature snapshots). |
| 3 | https://github.com/roapi/roapi | 3418 | 2026-03-25 | analytics | M | 3× | API layer over arrow/parquet datasets — useful as external query head, not core. |
| 4 | https://github.com/frankmcsherry/columnar | 197 | 2026-03-30 | store | S | 4× | Tight columnar serialization, low-overhead — good as IPC wire format between agents/workers. |
| 5 | https://github.com/CrunchyData/pg_parquet | 670 | 2025-11-09 | analytics | T | 2× | Parquet ↔ Postgres — only if PG path needed. |

## Q2 — Cranelift / JIT predicate

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 6 | https://github.com/bytecodealliance/wasmtime | 17999 | 2026-05-12 | jit | M | 6× | Embed cranelift via wasmtime for sandboxed user-defined filters/factors. |
| 7 | https://github.com/rust-lang/rustc_codegen_cranelift | 2048 | 2026-05-11 | jit | L | 2× | Compiler backend, not directly usable but reference for cranelift API. |
| 8 | https://github.com/bytecodealliance/cranelift-jit-demo | 741 | 2025-11-07 | jit | S | **8×** | Canonical reference for JIT-compiling small predicate ASTs (filter expressions, signal eval) directly to machine code via cranelift. |
| 9 | https://github.com/paradigmxyz/revmc | 272 | 2026-05-12 | jit | M | 4× | EVM JIT on cranelift — proves cranelift can JIT custom DSLs at production scale. Pattern source. |
| 10 | https://github.com/TheDan64/inkwell | 2920 | 2026-05-08 | jit | M | 3× | LLVM wrapper alt — heavier than cranelift but more optimization passes. |
| 11 | https://github.com/CensoredUsername/dynasm-rs | 827 | 2026-02-12 | jit | S | 4× | Inline asm-DSL for hot-path codegen — useful for tight SIMD orderbook updates. |
| 12 | https://github.com/qmonnet/rbpf | 1109 | 2026-02-06 | jit | M | 5× | eBPF VM+JIT in Rust — sandboxed user-defined filters, kernel-grade isolation. |

## Q3 — RaBitQ / FastScan / 1-bit quant

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 13 | https://github.com/VectorDB-NTU/RaBitQ-Library | 204 | 2026-05-12 | vec | M | **9×** | Official SIGMOD-2024/25 RaBitQ. 1-bit quant w/ theoretical error bound. Synapse-X already plans MRL-128 + RaBitQ tier; this is canonical impl. |
| 14 | https://github.com/lqhl/rabitq-rs | 14 | 2026-02-26 | vec | S | **7×** | Rust port of RaBitQ + IVF + MSTG (multi-scale tree graph). Direct cargo dep. |
| 15 | https://github.com/kemingy/rabitq | 10 | 2026-04-23 | vec | S | 5× | Alt Rust impl, simpler. Compare with #14 for code quality. |
| 16 | https://github.com/RyanCodrai/turbovec | 565 | 2026-05-02 | vec | S | **8×** | TurboQuant-based vector index in Rust with Python bindings. Newer than RaBitQ; bench head-to-head. |
| 17 | https://github.com/VectorDB-NTU/rabitq-turboquant-comparison | 9 | 2026-05-10 | vec/bench | T | 3× | Symmetric bench harness for RaBitQ vs TurboQuant — drop in to validate Synapse-X choice. |
| 18 | https://github.com/YARlabs/hyperspace-db | 113 | 2026-04-20 | vec | M | 3× | 1-bit quant vector DB w/ Lorentz/Poincaré — niche but unique hierarchical support. |

## Q4 — Online learner / FTRL / streaming SGD

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 19 | https://github.com/CastellanZhang/alphaFM | 904 | 2021-06-22 | signal | M | 4× | FM+FTRL multi-thread, C++. Reference for algorithm, port to Rust. |
| 20 | https://github.com/h2oai/datatable | 1880 | 2025-03-17 | analytics | L | 2× | Python tabular, peripheral. |
| 21 | https://github.com/comadan/FM_FTRL | 259 | 2016 | signal | M | 3× | Kaggle Avazu CTR FM-FTRL, Python ref impl. |
| 22 | (gap) Welford rolling stats — no high-star match | - | - | signal | T | 4× | Use `streaming-stats` crate or impl ~50 LOC; missing from search. |

## Q5 — Hilbert / Z-order curves

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 23 | https://github.com/cortesi/spacecurve | 527 | 2026-04-04 | store/index | T | **6×** | Active, well-known. Hilbert+Z-order LUT-based — drop-in for spatial/multidim feature locality. |
| 24 | https://github.com/becheran/fast-hilbert | 60 | 2026-02-13 | store/index | T | 5× | Pure LUT, very fast. Pairs with vortex for cluster-friendly row-ordering. |
| 25 | https://github.com/paulchernoch/hilbert | 74 | 2023-04 | store/index | T | 3× | Older, less maintained. |

## Q6 — Tick / Orderbook

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 26 | https://github.com/nkaz001/hftbacktest | 4050 | 2025-12-23 | book | M | **10×** | Full L2/L3 tick HFT backtester, queue positions, latencies, Binance/Bybit. Massive prior art for synapse-market book module. |
| 27 | https://github.com/exchange-core/exchange-core | 2512 | 2023-10 | book/router | L | 7× | Java LMAX-Disruptor matching engine w/ Adaptive Radix Tree price levels. Architecture blueprint. |
| 28 | https://github.com/chronoxor/CppTrader | 1022 | 2026-05-03 | book | M | 6× | C++ active orderbook+matching, low-latency primitives. |
| 29 | https://github.com/enewhuis/liquibook | 1457 | 2024-03 | book | M | 5× | Modern C++ order matching engine. |

## Q7 — Bloom / Cuckoo / Quotient filters

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 30 | https://github.com/tomtomwombat/fastbloom | 348 | 2026-03-01 | cache | T | **5×** | Fastest Rust bloom, concurrent, custom hasher. Drop in for dedupe/membership in ingest path. |
| 31 | https://github.com/axiomhq/rust-cuckoofilter | 295 | 2025-10-27 | cache | T | 5× | Cuckoo > Bloom for delete-support. |
| 32 | https://github.com/ayazhafiz/xorf | 164 | 2026-02-09 | cache | T | **6×** | Xor filters — smaller+faster than bloom/cuckoo, static sets. Best for read-mostly factor universes. |
| 33 | https://github.com/jedisct1/rust-bloom-filter | 272 | 2026-04-13 | cache | T | 3× | Stable classic impl. |
| 34 | https://github.com/arthurprs/qfilter | 24 | 2026-03-29 | cache | T | 4× | Rank-Select Quotient Filter in Rust — supports counting+resize. |
| 35 | https://github.com/seiflotfy/cuckoofilter | 1228 | 2024-07 | cache | T | 3× | Go, reference. |

## Q8 — Apple AMX / Accelerate

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 36 | https://github.com/apple/ml-stable-diffusion | 17851 | 2025-07 | - | - | n/a | Out of scope. |
| 37 | https://github.com/caiovicentino/apple-silicon-internals | 14 | 2026-03-26 | accel | S | **4×** | Reverse-engineered AMX/AMX2 + Metal4 ML pipeline. Reference for direct AMX matmul calls bypassing Accelerate. |
| 38 | https://github.com/Epistates/pmetal | 280 | 2026-05-08 | accel | M | 5× | Apple Silicon LLM inference w/ MLX+Metal — Rust binding patterns transferable. |
| 39 | (gap) `accelerate-sys` / `cblas-sys` | - | - | accel | T | **6×** | Direct cblas FFI — already common pattern, ~20 LOC. SimSIMD already wraps Accelerate. |

## Q9 — Adaptive Radix Tree

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 40 | (exchange-core, see #27) | 2512 | - | router | - | - | Has Java ART for orderbook price levels — port logic. |
| 41 | `rart` / `art-tree-rs` (search timed out, known crates) | - | - | router | S | **6×** | Use existing `art-tree` crate for orderbook price-level index. ~10× faster than BTreeMap for prefix-dense keys. |

## Q10 — Tabular ML / ONNX / TabPFN

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 42 | https://github.com/PriorLabs/tabpfn-client | (TBD) | 2026-05-12 | signal | T | 5× | Easy API access to TabPFN foundation model — wrap as factor-source. |
| 43 | https://github.com/PriorLabs/tabpfn-extensions | (TBD) | 2026 | signal | S | 4× | Ensembles + interpretability over TabPFN. |
| 44 | https://github.com/tracel-ai/cubecl | 2137 | 2026-05-12 | accel | L | **7×** | Multi-platform GPU compute lang for Rust (Burn ecosystem). Cross-platform GPU kernels for feature eng / inference. |
| 45 | (gap) `ort` (onnxruntime-rs) | - | - | signal | S | 5× | Standard ONNX inference; already mature. |

## Aux finds (high signal)

| # | Repo | Stars | Pushed | Slot | Effort | ×win | Notes |
|---|------|-------|--------|------|--------|------|-------|
| 46 | https://github.com/TimmyOVO/deepseek-ocr.rs | 2160 | 2026-02-21 | - | - | n/a | Rust DSQ quantization patterns — code-mining reference. |
| 47 | https://github.com/tinysearch/tinysearch | 2931 | 2026-02-03 | cache | T | 2× | WASM+bloom — niche. |
| 48 | https://github.com/madroidmaq/mlx-omni-server | 716 | 2026-05-09 | accel | S | 3× | OpenAI-compat MLX server — host-side inference. |
| 49 | https://github.com/Kaden-Schutt/hipfire | 368 | 2026-05-11 | accel | M | 3× | RDNA LLM inference Rust — AMD pattern source. |
| 50 | https://github.com/paradeb/pg_analytics | 536 | 2025-03 | analytics | T | 2× | DuckDB ⇆ Postgres bridge. |

---

## ROI summary (× win / effort)

Top by ROI (score = ×win / effort-weight; T=1, S=2, M=4, L=8):

1. **fast-hilbert** (#24): 5/1 = 5.0
2. **fastbloom** (#30): 5/1 = 5.0
3. **xorf xor-filter** (#32): 6/1 = 6.0
4. **spacecurve hilbert** (#23): 6/1 = 6.0
5. **vortex** (#1): 10/4 = 2.5 — but absolute upside biggest
6. **mmap-sync** (#2): 5/2 = 2.5
7. **rabitq-rs** (#14): 7/2 = 3.5
8. **turbovec** (#16): 8/2 = 4.0
9. **cranelift-jit-demo** (#8): 8/2 = 4.0
10. **hftbacktest** (#26): 10/4 = 2.5 — absolute upside biggest in book slot
11. **RaBitQ-Library** (#13): 9/4 = 2.25
12. **art-tree** crate (#41): 6/2 = 3.0

See TOP-10-INTEGRATIONS.md for deep notes + PR specs.
