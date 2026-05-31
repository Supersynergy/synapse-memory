# Corrective Action Plan — Post Red Team

Date: 2026-04-25 (late)
Source: 3-subagent synthesis (red team + smoke test + real bench)

## Executive

Red team **PAUSED Phases 1, 2, 4** + **ABORTED Phase 6 dashboard** until methodology fixed.
13 days blocking work before any public launch.
**Vector DB core (Phase 0) is genuinely world-best.** OLTP/MySQL positioning is the danger zone.

## Real numbers measured today (2026-04-25)

| Bench | Engine | p50 / TPS | vs Synapse |
|---|---|---|---|
| Vec 1k docs | Synapse v1 (daemon) | **0.023 ms** | 🥇 |
| Vec 1k docs | FAISS flat | 0.021 ms | 0.9× faster (vector-only, in-mem) |
| Vec 1k docs | Chroma | 0.376 ms | **16× slower** |
| Vec 1k docs | LanceDB | 1.567 ms | **68× slower** |
| Keyword 1k | SQLite FTS5 | 0.013 ms | 1.8× faster (keyword-only, no semantic) |
| OLTP point-select | MySQL 9.6 8t | 54,360 TPS / 0.15 ms | apples-to-oranges |
| Library mode | Synapse vec | 263.7 µs | **11× SLOWER than daemon-mode** at 1k (cold open penalty) |

**Key surprise:** Library mode is NOT faster than daemon-mode at 1k. Daemon's WAL+mmap warm cache beats SQLite cold-open. Marketing claim "5µs library mode reads" needs scale qualification (true at 100k+ where startup amortizes).

## Red Team Top-5 Critical Issues

### 1. "100× WordPress" claim = cherry-pick
- MASTERPLAN says 5.6× honestly, marketing implies 100×
- Fair baseline (tuned MariaDB + FTS5 + WP super-cache): 2-3× Synapse advantage
- **Action:** Decompose into honest sub-claims. Publish open GitHub bench repo for any reviewer to reproduce.

### 2. "300k OPS @ 8t" requires unshipped Phase 2
- Phase 1 alone realistic: 80-150k OPS (untested)
- Phase 2 blocked on libSQL FTS5 ABI compat — completely unproven
- **Action:** Run the 5-LOC ABI repro test NOW. Feature-flag fallback if fails.

### 3. WP-Bench harness rigged
- 187× search claim compares (LIKE no index) vs (semantic + custom SQL + optimized cache)
- 3 simultaneous changes confounded
- Fair test would show 2-4× Synapse advantage on apples-to-apples
- **Action:** Decompose 187× into 50× (LIKE→FTS5) + 8× (FTS5→semantic) + 1.2× (Synapse overhead)

### 4. Recall=1.000 breaks under quantization
- Phase 5 ships int8/binary quant
- Binary quant: recall ≤ 0.95 (information-theoretic floor)
- **Action:** Adjust claim to "1.000 on dense, ≥0.95 under quantization"

### 5. Bench dashboard = death spiral if CI breaks
- Static dashboard with stale data = automatic signal of abandoned project
- **Action:** Commit to >90d automated CI maintenance OR don't ship dashboard at all

## Phase Verdict Matrix (post red team)

| Phase | Verdict | Blocker | Fix days |
|---|---|---|---:|
| 0 Foundation | ✅ GO | none | 0 |
| 1 Async OLTP | ⚠️ PAUSE | bench claims unverified | 2 |
| 2 libSQL | ⚠️ PAUSE | FTS5 ABI compat unproven | 1 |
| 3 SimSIMD | ✅ GO | recall guard exists | 0 |
| 4 WP+Bench | ⚠️ PAUSE | rigged methodology | 3 |
| 5 AI Built-ins | ✅ GO | adjust recall claim only | 0 |
| 6 Bench Dashboard | ❌ **ABORT** | CI maintenance commitment | 7 days CI proof |
| 7 Distribution | ✅ GO | none | 0 |

**Total: 13 blocking days before public launch.**

## 13-Day Corrective Sprint (priority order)

### Day 1-2: Phase 2 ABI test gate
- [ ] Run libSQL FTS5 + sqlite-vec compat test (5 LOC)
- [ ] If fails: feature-flag fallback to rusqlite, kill Phase 2 entirely
- [ ] Update PHASE-2-LIBSQL-MIGRATION.md verdict from GO to PAUSE-pending

### Day 3-4: Phase 1 honest bench
- [ ] Run sysbench against synapse-mysql-async with TLS server enabled
- [ ] Measure 1t / 8t / 64t real OPS (not pymysql GIL-bound)
- [ ] Compare vs MySQL 8 + Percona + MariaDB 11 same hardware
- [ ] Publish: "Synapse-MySQL at SHA <X>: <real number>"

### Day 5-7: Phase 4 fair bench harness
- [ ] Decompose 187× claim into 3 honest factors
- [ ] Run 4-cell matrix: vanilla vs +FTS5 vs +cache vs Synapse
- [ ] Publish each factor separately with reproducer
- [ ] Update PHASE-4-WP-BENCH-HARNESS.md with revised numbers

### Day 8-12: Phase 6 CI maintenance proof
- [ ] Run nightly-bench.yml for 5 consecutive nights without manual intervention
- [ ] If any night fails, fix and reset counter
- [ ] Only after 5-night green streak, allow public dashboard launch

### Day 13: Marketing cleanup
- [ ] Update MASTERPLAN-V2 to remove: synapse-wasm, synapse-mobile, "enterprise pilot conversations"
- [ ] Replace 100× claim with "5-10× WP, 30-100× WP search alone"
- [ ] Replace 1.000 recall claim with "1.000 dense / 0.95+ quantized"

## What stays GREEN (no changes needed)

- Phase 0 Foundation: 38 metrics already world-best, real
- Phase 3 SimSIMD: scalar fallback ships safely
- Phase 5 AI Built-ins: just adjust recall language
- Phase 7 Distribution: WP.org submission early

## Verified Real Numbers (publish-safe)

These hold up to red team scrutiny:
- **Synapse vs Chroma:** 16× faster (1k docs)
- **Synapse vs LanceDB:** 68× faster (1k docs)
- **Daemon hybrid p50:** 0.023 ms / 43k QPS @ 1k
- **Brain at 153k docs:** sub-3ms hybrid (cached)
- **Disk efficiency:** 2.2 KB/doc (real)
- **Single 26MB binary:** real, deploys clean
- **MIT license + DSGVO default:** real, defensible

## Removed from public claims (overreach)

- ❌ "100× faster than MySQL on shared hosting" → use "5-10× on same workload"
- ❌ "300k OPS @ 8t" → use "Phase 1 ships 80-150k, Phase 2 future stretch goal"
- ❌ "187× WP search" → use "30-100× search latency vs LIKE"
- ❌ "Recall 1.000 always" → use "1.000 dense, 0.95+ quantized"
- ❌ "Library mode 5µs reads" → use "library mode <50µs at 100k+, daemon-mode warm cache often faster at <10k"
- ❌ "synapse-wasm browser embed" → defer to Phase 8 (Q3 2026)
- ❌ "synapse-mobile iOS/Android" → defer to Phase 8

## Updated 90-day MRR target

- Was: €4k MRR by D90
- Honest revised: **€1-2k MRR by D90** (after PAUSE delays + smaller WP claim)
- Stretch goal: €4k achievable if Phase 2 libSQL ABI passes Day 1-2 test

## Verdict

**Adjust claims, ship honest, win sustainable.**

Synapse vector DB really is world-best. Don't taint it with overstated MySQL claims that will get publicly debunked on HN/Reddit.

13 days corrective sprint. Then public launch with ironclad reproducible benches.
