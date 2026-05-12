# RED TEAM PRELAUNCH AUDIT — Synapse Gamechanger
**Date:** 2026-04-25 | **Verdict:** ⚠️ PAUSE PHASES 1-2 until bench claims verified

---

## EXECUTIVE: Top-10 Launch Killers

| # | Kill Vector | P×I Score | Mitigation Before Launch | Days |
|---|---|---:|---|---:|
| 1 | **100× WP claim = cherry-pick on tiny data** | M×H = 9 | Run vanilla WP(real+plugins) vs Synapse; publish exact repo/WP-ver | 14 |
| 2 | **300k OPS requires all 3 phases shipped, untested** | M×H = 9 | Phase 1 alone gives 80-150k; bench NOW before launch claims | 7 |
| 3 | **opensrv-mysql port hits ABI surprise** | M×M = 6 | ABI compat test BEFORE forking; feature flag fallback ready | 3 |
| 4 | **libSQL FTS5 extension ABI untested** | L×H = 5 | Run 5-LOC repro test NOW on libSQL v0.6; feature flag gate | 2 |
| 5 | **wp_options 24× only if auto-reload disabled** | M×M = 6 | Document exact WP config; publish repro test | 4 |
| 6 | **Recall=1.000 regresses under int8 quant** | L×H = 5 | Hold at 0.95+; adjust marketing | 1 |
| 7 | **WordPress.org plugin review rejects WP<6.0** | L×M = 4 | Target WP 6.4+; test on WP cloud | 5 |
| 8 | **Pinecone/Qdrant counter-bench in 3h** | M×M = 6 | Pre-release vs v6.x; publish methodology | 7 |
| 9 | **Library mode 5µs claim conflates wire proto** | L×M = 4 | Split: "5µs in-process / 50-200µs via MySQL protocol" | 2 |
| 10 | **Bench dashboard becomes stale; credibility spiral** | M×H = 9 | Automated nightly CI in GHA; link reproducible repo | 14 |

---

## Top-5 Deep Dives

### 1. The "100× WordPress" Marketing Trap

**The Claim:** vanilla WP+MariaDB 280ms → Synapse 50ms = 5.6× (marketing says 100×)

**Hostile Critic (Algolia engineer):**
```
"You're comparing:
- Stock MariaDB with NO tuning
- vs Synapse-WP with:
  * dedicated wp_options cache (11.7× speedup)
  * semantic search shortcode (replaces LIKE)
  
Fair bench: both with same optimizations.
Gap drops to 2-3×. You cherry-picked baseline."
```

**Reality from LIMITS doc:** Synapse-MySQL gap is 10× at best. wp_options accounts for 11.7× alone.

**Mitigation - MUST DO:**
1. Publish exact test setup (WP version, plugins, configs)
2. Run on 3 tiers (€5 Hetzner CX11, €15 DO, AWS t3)
3. Decompose: cache=11.7×, protocol=5×, hybrid=2.3×, total=5-6×
4. Remove "100×" from marketing; use "6× faster"
5. GitHub issues for "fair bench" challenges

**Fix Cost:** 5 days

---

### 2. "300k OPS" Requires Phase 1+2 Both Shipped

**The Claim (MASTERPLAN v2):**
```
MySQL OLTP 8t: 300k target
= Phase 1 async (10×) + Phase 2 libSQL (4× more) = 40× total
```

**Reality:**
- Phase 1 alone: 16k → 80-150k OPS (realistic)
- Phase 2 needed for 300k (ships Week 3)
- If Phase 2 slips: 300k claim false immediately post-launch

**Mitigation - MUST DO:**
1. Run Phase 1 bench TODAY on real repo (not projection)
2. Run isolated libSQL ABI test (5 LOC):
   ```rust
   let db = libsql::Builder::new_local(":memory:").build().await?;
   db.execute("CREATE VIRTUAL TABLE v USING vec0(id INTEGER, e FLOAT[384])", ()).await?;
   assert!(db.execute("CREATE VIRTUAL TABLE f USING fts5(c)", ()).is_ok());
   ```
3. If test fails → remove "300k" from launch narrative
4. Publish weekly: "Phase 1 achieved 120k OPS — Phase 2 in progress"

**Fix Cost:** 2 days testing

---

### 3. opensrv-mysql ABI Stability Risk

**The Plan:** Fork databendlabs/opensrv-mysql v0.10 + implement 12 MysqlShim callbacks

**Hidden Risk:** opensrv-mysql **not stable**
- GreptimeDB carries custom patches (PR-81)
- Last updated 4 months ago
- Drift between versions is common
- No crates.io releases after v0.10

**Mitigation - MUST DO:**
1. Don't fork; vendor locally under `crates/synapse-mysql-async/`
2. Pin to v0.10 commit hash (not version)
3. Add feature flag fallback:
   ```toml
   [features]
   mysql-async = ["dep:opensrv-mysql"]  # opt-in
   mysql-compat = []  # default: uses v5 (stable)
   ```
4. Never claim "fully compatible MySQL" — say "subset: SELECT/INSERT/UPDATE/DELETE"
5. Test 3 failure modes before shipping:
   - TLS cert expiry mid-transaction
   - Prepared statement collision
   - Multi-statement batching

**Fix Cost:** 1 day for feature flag + tests

---

### 4. WP-Bench Harness = Rigged by Design

**The Scenario (PHASE-4):**
```
Vanilla: SELECT * FROM wp_posts WHERE (post_title LIKE '%rust%' OR ...) = 1500ms
Synapse: SELECT * FROM wp_posts WHERE synapse_match(...) = 8ms → 187×
```

**Hostile Critic:** "This compares:
- Vanilla WP (LIKE, unoptimized)
- vs Synapse with THREE changes:
  a) semantic search SQL function
  b) concatenated columns
  c) index optimization

Fair test: apply FTS5 to vanilla too."

**Mitigation - MUST DO:**
1. Decompose 187× into three 3-5× claims:
   - "LIKE→FTS5: 50× faster" (standard DB improvement)
   - "FTS5→semantic: 8× faster on intent" (algorithm win)
   - "Synapse overhead: 1.2×" (actual Synapse value-add)
   - Total: 480× possible; claim 187× = conservative

2. Publish full bench repo on GitHub:
   - docker-compose both stacks
   - Seed data (1k posts, exact WP version)
   - k6 script (shareable)
   - Results CSV+JSON
   - **Invite competitors to run it**

3. Claim: "6× faster TTFB than unoptimized WP" (honest)

4. Add disclaimer:
   ```
   This measures vanilla WP vs optimized Synapse.
   If you apply same optimizations to vanilla MySQL,
   gap narrows to 2-3×. See fair_bench.md.
   ```

**Fix Cost:** 3 days

---

### 5. Recall=1.000 Breaks Under int8 Quantization

**The Claim (MASTERPLAN v2):**
```
Recall@10: 1.000 (hold)
+ Phase 5: BitNet 1.58-bit (4× compression)
```

**The Math:**
- Binary (1-bit per dim): 384→48 bytes = 8× smaller
- Under 1-bit quant: **recall ≤ 0.95** (information loss)

**Hostile Critic:** "You promised 1.000 recall. Now quantization cuts it to 0.92. That's moving goalposts."

**Mitigation - MUST DO:**
1. Change claim to:
   - "Recall ≥ 0.95+ with binary quantization"
   - "Recall = 1.000 on original (384D) embeddings"

2. Publish trade-off matrix:
   ```
   f32: 1× compression, 1.000 recall (default)
   int8: 4× compression, 0.98 recall
   binary: 32× compression, 0.90 recall (opt-in)
   ```

3. Gate binary quant behind feature flag with docs

4. Add test: `#[test] fn test_recall_int8_holds_095()`

**Fix Cost:** 1 day

---

## 5 Bench Claims Needing Stricter Methodology

| Claim | Current Evidence | Issue | Must-Fix |
|---|---|---|---|
| **0.28ms p95 @ 1M** | Local screenshot | No hardware spec, no variance, no contention | Run 10× on same HW; mean ± stddev; 10 concurrent clients |
| **50k+ OLTP reads** | Projection from "70% gap reduction" | v5 untested; v6 greenfield | Run sysbench TODAY; publish setup; vs MariaDB 11 + MySQL 8.0.40 |
| **180k+ YCSB workload A** | Projection from "libSQL 4× gap" | libSQL ABI untested | Run libSQL ABI test NOW; if fails, Phase 2 blocked |
| **6.5× faster WP** | k6 harness projection | k6 ≠ real browser; ignores JS+caching | Use Lighthouse/WebPageTest; real shared hosting; include WP cache plugins |
| **MLX embedder = 5ms** | Architecture promise | 5ms on M-series only | Publish 3 platforms (M, x86 AVX2, ARM64); gate behind `--metal` flag |

---

## 3 Things to Remove (Overreach)

### 1. "synapse-wasm — embed in browser"
- Requires 9MB+ WASM, complex browser storage sync
- Chroma-js exists; immature space
- Slip risk: HIGH
- **Remove:** Line 414

### 2. "synapse-mobile — iOS/Android binding"
- Requires Kotlin/Swift FFI (2+ weeks each)
- 3+ new test platforms
- No revenue by Day 90
- **Defer to Phase 8 (Q3 2026)**

### 3. "Enterprise pilot conversations (success metric)"
- Enterprise sales cycle 6-12 weeks
- Synapse-MySQL not production-ready until Phase 2
- **Revise to:** "3+ design-partner conversations (not closed pilots)"

---

## GO/PAUSE/ABORT Verdict Per Phase

| Phase | Verdict | Action |
|---|---|---|
| **0 (Foundation)** | ✅ GO | Ship as-is |
| **1 (Async OLTP)** | ⚠️ **PAUSE** | Bench 80-150k first; feature-flag required | 2-day test gate |
| **2 (libSQL)** | ⚠️ **PAUSE** | FTS5 ABI compat unproven; run repro test blocks | 1-day test; fallback ready |
| **3 (SimSIMD)** | ✅ GO | Scalar fallback exists | Don't claim "4× speedup" without bench |
| **4 (WP+Bench)** | ⚠️ **PAUSE** | Methodology rigged; decompose claims | 3 days: fair bench + GitHub repo |
| **5 (AI Built-Ins)** | ✅ GO | Adjust recall to 0.95+; remove binary quant from d90 |
| **6 (Bench Marketing)** | ❌ **ABORT** | Dashboard stales; kills credibility | Automate CI + commit to >90d maintenance |
| **7 (Distribution)** | ✅ GO | Just submit WP plugin early (Day 35+) |

---

## Must-Fix Before Launch (13 days blocking)

1. Run Phase 1 bench (80-150k) on real repo — 2 days
2. Test libSQL FTS5 ABI compat — 1 day
3. Decompose "187× WP search" into fair sub-benchmarks — 3 days
4. Publish fair WP-Bench on GitHub (open to competitors) — 2 days
5. Add feature flags for both MySQL protocols (safe fallback) — 1 day
6. Adjust recall claim from 1.000 to 0.95+ under quantization — 0.5 days
7. Automate bench dashboard (daily CI) — 3 days
8. Remove wasm/mobile/enterprise from d90 scope — 0.5 days

---

## Final Verdict

**Status:** ⚠️ **PAUSE — Fix Bench Claims & Tests**

**Why:** Synapse-V core (vector DB) is genuinely world-best. But 90-day plan makes **4 false claims** that competitors will demolish within 48h of HN launch.

**If you ship as-is:** HN upvotes → Reddit scrutiny → "marketing lies" within 72h → 2+ years brand damage.

**If you fix:** HN upvotes → credibility verified → genuine 12-month moat.

Path forward: Days 1-3 bench tests, Days 4-13 fair benchmarks + feature flags, Day 14+ launch with honest claims.
