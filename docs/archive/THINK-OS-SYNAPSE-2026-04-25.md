# THINK-OS — Synapse Gamechanger Orchestration (ultrathink)

Date: 2026-04-25
Layer: META — coordinates Masterplan-V2 execution via Agent Teams + superml + thinkrich.
Goal: maximize compound effect of all phases via parallel subagent orchestration + adaptive ML feedback loops.

---

## I. THE COMPOUND THESIS (why ultrathink wins linear)

Linear plan delivers Phase 1 (10×), then Phase 2 (4× write), then Phase 3 (5× SimSIMD).
Compound plan delivers **all phases parallel** via subagent teams, then layers **adaptive ML routers** on top so each query picks the optimal path.

| Approach | Phase 1 alone | Phase 1+2 | Phase 1+2+3 | + adaptive router |
|---|---:|---:|---:|---:|
| Linear sequential | 10× OPS | 40× | 200× | 200× |
| Parallel + ML compound | 10× | 40× | 200× | **260-280× via path selection** |

The 30-40% extra comes from superml routing each query to its optimal path (Lex when keyword-heavy, Vec when semantic, HybridW when both, with fallback per measured confidence).

---

## II. AGENT TEAM STRUCTURE (6 teams, 90 days)

### Team Alpha — Phase 1: Async OLTP Wire (Days 1-14)
| Subagent | Type | Mission |
|---|---|---|
| α1 | implementer | Port opensrv-mysql shim (`synapse-mysql-async/src/shim.rs`) |
| α2 | researcher | Study databend + greptime AsyncMysqlShim patterns |
| α3 | reviewer | Build A/B test harness vs v5 |
| α4 | implementer | tokio-rusqlite wiring + spawn_blocking pattern |
| α5 | researcher | TLS/auth compat path |
**DoD:** sysbench oltp_point_select 8t ≥ 100k QPS, A/B passes 5× over v5.

### Team Beta — Adaptive Query Router (Days 1-21, parallel to Alpha)
| Subagent | Type | Mission |
|---|---|---|
| β1 | researcher | Feature engineering (q-len, lang, IDF, ctx) |
| β2 | implementer | DuckDB query-log table + nightly CatBoost trainer |
| β3 | implementer | Rust inference module (`synapse-core/src/turbo/router_v2.rs`) |
| β4 | reviewer | Shadow-mode logger + A/B harness |
**DoD:** router predicts best mode 92%+ accuracy, p95 latency drops 20%+ vs Hybrid-default.

### Team Gamma — WordPress + WP-Bench (Days 22-49)
| Subagent | Type | Mission |
|---|---|---|
| γ1 | researcher | WP hot-path pattern mining (autoload, postmeta JOIN, search LIKE) |
| γ2 | implementer | synapse-wp 0.2 (Woo + shortcodes + admin bench page) |
| γ3 | implementer | wp-bench harness (TTFB home/single/search/admin) |
| γ4 | reviewer | Compare runner: vanilla WP+MariaDB vs WP+Synapse |
**DoD:** TTFB home <50ms, search <8ms, 10× faster than vanilla on €5/mo Hetzner.

### Team Delta — libSQL Migration (Days 15-28, parallel to Alpha+Beta)
| Subagent | Type | Mission |
|---|---|---|
| δ1 | researcher | FTS5 + sqlite-vec ABI compat with libsql |
| δ2 | implementer | Backend trait + libsql impl behind feature flag |
| δ3 | implementer | Turso edge replication wiring |
| δ4 | reviewer | YCSB workload A 8t ≥ 180k OPS validation |
**DoD:** BEGIN CONCURRENT works, write-mix 4× faster, edge replica deploy <5min.

### Team Epsilon — SimSIMD Everywhere (Days 29-49)
| Subagent | Type | Mission |
|---|---|---|
| ε1 | researcher | Map every scalar hot-path candidate (BM25, HNSW, RRF, JOIN-hash, COUNT/SUM) |
| ε2 | implementer | Wire simsimd kernels per path |
| ε3 | reviewer | Recall regression guard (must hold 1.000 on dense, 0.95+ on quant) |
**DoD:** hybrid p50 <0.5ms at 137k brain.

### Team Zeta — AI Built-Ins + MLX (Days 50-70)
| Subagent | Type | Mission |
|---|---|---|
| ζ1 | implementer | synapse-metal v0.1 → live MLX backend for embedder |
| ζ2 | implementer | L3 ColBERT rerank cache (lightonai/pylate ref) |
| ζ3 | implementer | SQL functions: synapse_match, synapse_related, synapse_answer |
| ζ4 | researcher | BitNet 1.58-bit quant viability for 4× embed compression |
**DoD:** SELECT synapse_related(post_id) returns 5 hits in <5ms; MLX 6× faster embed.

---

## III. SUPERML INTEGRATION MAP (where ML compounds Phase wins)

| Use Case | superml Algo | Training Data | Inference Budget | Phase |
|---|---|---|---|---|
| Query mode router | CatBoost (cat-heavy) | DuckDB query log | <100µs | 2B |
| Cache pre-warm | LightGBM | hit-rate logs | <50µs | 4 |
| Anti-spam WP search | EBM (interpretable) | query stream | <500µs | 4 |
| Embed-model selector | TabPFN v2 (small data) | doc-type→recall | <1ms | 5 |
| Antiban stage classifier | CatBoost (existing) | host history | <50µs | (existing) |
| Auto-tune per HW | XGBoost | bench-result logs | offline | 3 |
| Slow-query alert | NGBoost (uncertainty) | latency time-series | online | 7 |

### The self-improving loop
```
[Query] → [Fingerprint] → [superml predict] → [chosen path]
                                                    ↓
                                              [latency log]
                                                    ↓
                                              [DuckDB ATTACH]
                                                    ↓
                                              [nightly retrain]
                                                    ↓
                                              [A/B shadow test]
                                                    ↓
                                              [promote if win] ──→ back to start
```
**Outcome:** Synapse becomes faster every night without human input. Universal-Adapter pattern.

---

## IV. THINKRICH (revenue track) — €/Phase mapping

| Phase | Direct Revenue | Indirect | Cumulative MRR by Day 90 |
|---|---:|---|---:|
| 1 Async OLTP | €0 | benchmark cred | €0 |
| 2 libSQL | €0 | edge story | €0 |
| 3 SimSIMD | €0 | speed claims | €0 |
| 4 WP + WP-Bench | **€500-1k MRR** | 10 design partners | €500-1k |
| 5 AI built-ins | **+€500 MRR** | replaces Algolia €500/mo | €1k-1.5k |
| 6 Bench launch | indirect (HN top-10 = 100 leads) | virality | €1k-2k |
| 7 Distribution | **+€500-2k MRR** | Synapse Cloud + agency tier | **€2k-4k MRR by D90** |

### Pricing tiers (final)
| Tier | €/mo | Features | Target | Conversion |
|---|---:|---|---|---:|
| Free | 0 | 1 site, 100k docs, community | hobbyist | n/a |
| Pro | 29 | 5 sites, 1M docs, email support | small biz / freelance | 5% of free |
| Agency | 99 | 25 sites, 10M docs, priority | agency / consulting | 10% of pro |
| Enterprise | 499 | unlimited, SLA 4h, white-label | enterprise / SaaS | 5% of agency |
| Synapse Cloud | 49+ | managed Fly.io, auto-scale | "no DevOps" tier | 20% of pro+ |

### Anchor pricing
- Algolia: $500/mo for 100k records → Synapse 17× cheaper at agency tier
- Pinecone: $70/mo starter → Synapse 2× cheaper at pro tier
- Elasticsearch managed: $90+/mo → Synapse 3× cheaper

### TAM (thinkrich angle)
- WP plugins: 65k actively maintained, ~500/yr cross €100k+ ARR. Synapse-WP captures top 1% = €1M/yr.
- Embedded SQL devs (mobile, desktop): ~5M devs × $2/yr lib license avg = $10M TAM.
- Algolia replacements: 50k current Algolia customers × 30% switch potential = €5M/yr.
**90d goal:** €4k MRR = €48k ARR foundation, 0.005% of TAM, plenty headroom.

---

## V. PARALLEL EXECUTION — Daily Cadence

### Daily standup (15min)
- Each team posts: yesterday-done, today-plan, blockers
- Auto-aggregated to `~/.synapse/standup-2026-04-25.md`

### Nightly cron (00:00-06:00)
- superml retrains all routers on day's logs
- bench-tracker runs full bench suite, posts to dashboard
- regression-sentinel alerts on >5% perf drop
- doc-sync auto-updates README from code

### Weekly sync (Friday 17:00)
- All teams demo PRs
- Bench dashboard reviewed
- Phase progress vs DoD
- Promote shadow-routers if A/B win

### Bi-weekly retro
- Decide phase pivots
- Adjust agent team composition
- Publish public bench update

---

## VI. CRITICAL ULTRATHINK INSIGHTS

### 1. Marketing IS development
Every benchmark publication = lead funnel. Bench-marketing is not a Phase 6 afterthought — it runs in parallel from Day 1. Live dashboard at synapse.sh/bench updated nightly = continuous credibility.

### 2. Single-binary is the moat
MySQL needs a DBA, Postgres needs a service mesh, Synapse fits in a `cargo install`. This is THE category-defining property. Every phase must protect it.

### 3. Library mode is the Trojan horse
`cargo add synapsed` → developers embed Synapse in their app → become customers when they self-host. Distribution > marketing.

### 4. WordPress is the beachhead, not the destination
WP gives 43% of web reach, but the real prize is replacing Postgres+Redis+Elasticsearch+Pinecone in modern Rails/Django/Laravel apps. WP is just the proof.

### 5. Adaptive ML is unfair advantage
Once superml router is shipping, every customer's traffic feeds back to make the system smarter. Network effect that closed-source can't match.

### 6. Compound speeds compound revenue
1ms hybrid search → developer can build apps that wouldn't ship at 50ms. New apps = new use cases = new TAM.

---

## VII. AGENT INVOCATION CHEATSHEET

```bash
# Spawn Team Alpha lead
Agent({
  description: "Phase 1 async MySQL coordination",
  subagent_type: "implementer",
  prompt: "Drive Team Alpha. Read MASTERPLAN-V2-GAMECHANGER. Port opensrv-mysql shim. PR daily. Definition of done: sysbench 100k OPS @ 8t."
})

# Spawn Team Beta lead
Agent({
  description: "Adaptive router design",
  subagent_type: "researcher",
  prompt: "Read THINK-OS Section III. Design superml-CatBoost query router. Output PHASE-2B-ADAPTIVE-ROUTER.md."
})

# Spawn parallel: Team Gamma WP-Bench harness
Agent({
  description: "WP-Bench scenario harness",
  subagent_type: "implementer",
  prompt: "Build wp-bench harness. 1k posts seed. Scenarios: home, single, search, admin. Output to crates/synapse-wp-bench/."
})
```

Coordination via `~/.synapse/teams-state.json` — each team writes status hourly.

---

## VIII. MEASUREMENT — "Are we winning?"

### Daily dashboard metrics (auto-collected)
1. Synapse hybrid p50 (target: monotone decreasing)
2. MySQL-async OPS @ 8t (target: 100k by D14, 300k by D30)
3. WP-bench TTFB home (target: <50ms by D49)
4. Recall@10 (target: hold 1.000)
5. RAM/M docs (target: decreasing toward 80MB)
6. GitHub stars (target: 5k by D90)
7. WP plugin installs (target: 100 by D90)
8. MRR (target: €4k by D90)
9. PR throughput (target: 5+/day across teams)
10. Bench publication count (target: 1+/week)

### "Gamechanger" promotion gate (Day 90)
80/100 params world-best AND HN top-10 launch AND 1 mainstream press cite AND €4k+ MRR = certified gamechanger. Otherwise iterate.

---

## IX. KNOWN UNKNOWNS (for next ultrathink turn)

1. Will libSQL FTS5 ABI compat hold at scale? Need empirical test.
2. opensrv-mysql v0.7 vs v0.10 (databend git) — which to standardize on?
3. WP plugin review timeline — WP.org SVN can take 2-4 weeks?
4. ColBERT MLX port effort — does it exist or do we port?
5. Synapse Cloud — Fly.io vs Railway vs Hetzner managed? cost/perf tradeoff.

Each of these = 1 researcher subagent if blocking.

---

## X. NEXT 24h CONCRETE EXECUTION

1. ✅ THINK-OS doc written (this file)
2. ⏳ Continue shim.rs (Team Alpha α1 active in this conversation)
3. ⏳ Spawn Team Beta researcher (router design) — DONE in this turn
4. ⏳ Cherry-pick wp-bench-3way fixes onto main
5. ⏳ Build + smoke test synapse-mysql-async crate
6. ⏳ A/B harness skeleton (Team Alpha α3 next)
7. ⏳ Open issue tracker entries for all 6 teams
8. ⏳ Set up `~/.synapse/teams-state.json` for coordination
