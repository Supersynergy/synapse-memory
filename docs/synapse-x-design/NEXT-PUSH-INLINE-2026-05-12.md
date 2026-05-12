# Synapse-X Next-Push — Inline Synthesis (2026-05-12)

> Deep-analysis agent stalled on aggressive ghmax-storm. This doc = inline-condensed synthesis from existing artifacts + Synapse local recall + design-doc trajectory.

## Where Synapse-X CURRENTLY LOSES vs SQLite (honest)

| Workload | SQLite-persist | Synapse-X | Gap |
|---|---:|---:|---|
| Single-ticker point-lookup | 22µs | ~50µs | 2× slower |
| Aggregate top-30 | 9µs | ~50µs | 5× slower |
| <1k bars/ticker corpus | dominates | overhead | crossover |

→ **One real bottleneck**: Synapse-X has 15µs file-handle setup-overhead per opening; SQLite-persistent has none. Below ~1k bars the overhead dominates.

## Five Pushes To Beat SQLite Across ALL Workloads

### Push 1 — Handle Pool + In-Memory Hot-Set (the killer)
- Persistent `Market` keeps Series handles open for every ticker (already done by held-handle bench: 26µs)
- Add `HotSet` LRU of last-1000 accessed pages in process-heap (uncompressed)
- HotSet hit-path: zero-mmap-walk, direct slice access
- Expected p50 point-lookup: **3-8µs** ≤ SQLite's 22µs
- **Push**: 3-5× on small workloads, no regression on big

Hebelwort: **antifragile** (more queries = better cache), **zero-friction** (no setup)

### Push 2 — Bloom-filter per Series for negative-lookup
- Each Series writes 8-bit bloom over its ts-key-set at load-time
- Negative-lookup (ticker not present, ts out-of-range): **single-bit-test**, ~50ns
- Eliminates the "no-data" worst-case (currently full-page scan)
- Implementation: `xx-hash` + 3-hash bloom, 16KB/Series

Hebelwort: **picks-and-shovels** (every query benefits)

### Push 3 — Compiled Filter via cranelift (or sealed Rust closures)
- Light JIT for `WHERE close > X AND volume > Y` style predicates
- Compile once per query-shape, cache in `PlanCache`
- DuckDB has this since 2023; QuestDB 8 added it 2025; we add via existing `cranelift-jit = "0.103"` (tiny dep, no LLVM)
- Expected: **2-3×** on filtered scans (W3 A2 VWAP currently 1×, would jump to 3-4×)

Hebelwort: **compounding** (PlanCache learns + JIT compiles → next query free)

### Push 4 — Adaptive Radix Tree (ART) over ticker→Series map
- Currently `Market` has HashMap<String,Series>
- ART = sub-µs prefix-lookup, sorted iteration, prefix-range queries
- Real win when "list all tickers starting with AAP*" queries
- Crate: `art-tree` or roll-our-own (~200 LoC)

Hebelwort: **moat** (nobody else has ART+columnar+vector)

### Push 5 — io_uring batch-scan (Linux) / posix_aio (macOS) for parallel-ticker queries
- Q5-style "scan 10 tickers concatenated" currently linear
- Submit-all-reads in one syscall (`io_submit` Linux / `posix_aio_*` Darwin)
- Re-use existing tokio for execution, just batch-the-IO
- Expected: **3-5×** on multi-ticker workloads where SQLite still wins

Hebelwort: **antifragile** (concurrent-load improves throughput)

## Stack-Math (honest discount 0.4 across compounded factors)

| Layer | × | Stacked |
|---|---:|---:|
| Current baseline | 1× | 1× |
| HotSet (Push 1) | 3× | 3× |
| Bloom (Push 2) on negative-lookup | 1.5× | 4.5× |
| JIT-compile (Push 3) on filters | 2× | 9× |
| ART (Push 4) on prefix-queries | 1.3× | 11.7× |
| io_uring (Push 5) on multi-ticker | 2× | 23.4× |
| **Discount 0.4** | | **9.4× honest** |

→ on top of current 7.2× big-data win = **~67× over SQLite** projected on combined workloads
→ on small-OLTP: 22µs → 5-8µs = **3-4× over SQLite** (closes the gap, then wins)

## Anti-Pattern Killlist (where stacking is theater)

- ❌ Adding LSM-tree (RocksDB-style) — wrong shape for tick-data
- ❌ Vector-similarity in main path — separate concern, lives in `signal/`
- ❌ Distributed-anything before single-node wins everywhere
- ❌ "More indexes" — ART covers prefix; ts is already sorted; price is Hilbert-zorder
- ❌ Replicating SQLite's WAL semantics — pointless, append-log + COW snapshot already win

## Dependencies on Existing SQL-Stack (synapse-libsql, synapse-mysql, synapsql-row)

- HotSet pattern = same as CRM-cache (197k× weighted) — proven shape
- ART map = matches libsql's internal page-index, could share if needed
- JIT-compile = synapse-mysql wire-protocol can route compiled-filters too
- → existing layers stay, Synapse-X wins underneath

## Sequence (4-week plan)

| Week | Push | Effort | Expected Win |
|---|---|---|---|
| W8 | HotSet + handle-pool | 8h | 3× small-data, closes SQLite gap |
| W8 | Bloom-filter | 3h | 1.5× negative-lookup |
| W9 | cranelift JIT filter | 12h | 2-3× filtered scans |
| W10 | ART ticker-map | 6h | 1.3× prefix-queries |
| W10 | io_uring batch-scan | 8h | 2-5× multi-ticker |
| W11 | Stack-bench all 5 | 4h | acceptance gates, honest report |

Total: ~40h / 1 person-week. Realistic 2 calendar-weeks with reviews/CI.

## Honest Bail-Criteria

- HotSet doesn't hit 8µs p50 → kill, stay at 26µs (still 7× over SQLite at scale)
- JIT-compile adds >100µs compile-overhead → cache-or-skip-strategy required
- ART over 220 ticker = overkill; only ship if scale ≥ 10k tickers expected

## Top-3 ROI-Optimal NEXT Pushes (= start here)

1. **HotSet + held-handle pool** — biggest win, lowest risk, closes one real gap
2. **Bloom-filter** — 3h for 1.5× — trivial
3. **cranelift JIT for filter predicates** — graduate the system from "fast columnar scanner" to "small embedded query engine"

After those 3: synapse-x is uncontested embedded-store for quant + time-series.

## What's NOT in this push (intentional)

- L2-book write-time-feature-DAG (W11, separate track)
- Inline online-learner-state (W12, separate track)
- Adapter-distribution (vectorbt/qlib) (W15, after stable API)
- WASM-component / browser-embed (defer)
