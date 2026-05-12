# superml — Synapse Optimization Opportunities

Date: 2026-04-25 · ML adaptive routing analysis on top of MASTERPLAN-V2

## Top 7 superml use cases for Synapse, ranked by expected ROI

### 1. Query mode router (Phase 2B) — ALREADY DESIGNED ✅
- **Algo:** CatBoost multiclass (Lex/Vec/Hybrid)
- **Features:** 12 (q-len, IDF, embedding-entropy, …)
- **Inference:** <100µs
- **Gain:** +15-30% p95 on real traffic
- **Status:** spec ready, awaits Phase 1 query logs

### 2. Result-cache pre-warm predictor — NEW
- **Algo:** LightGBM (binary classification: will this query hit cache in next 60s?)
- **Features:** query hash, last-seen, hour, hit-rate-trailing-1h, brain-size
- **Action:** if predicted hit, pre-compute + push to LRU before request
- **Inference:** <50µs
- **Gain:** Cache hit-rate 60% → 85% → p50 sub-ms on cache-warm queries
- **When viable:** after Phase 1 cache logs accumulate

### 3. Embed-model selector per doc-type — NEW
- **Algo:** TabPFN v2 (small training data, 100-1000 examples)
- **Input:** doc-type cluster (code, news, blog, social, scientific), language, length
- **Output:** which embedder gives best recall/cost (BGE-small EN, E5 multilingual, code-specific)
- **Gain:** +5% recall on heterogeneous corpora at no latency cost
- **When viable:** after multilingual eval data exists

### 4. Anti-spam classifier for WP search — NEW (Phase 4)
- **Algo:** EBM (interpretable — needed for content moderation)
- **Features:** query length, special chars, repetition score, IP velocity, time-of-day, user-agent fingerprint
- **Action:** rate-limit or reject obvious spam queries
- **Gain:** prevents search-API DDoS on public WP sites
- **When viable:** post WP plugin launch, traffic accumulates

### 5. Embed-cache priority (Bandit) — NEW
- **Algo:** Thompson sampling bandit
- **Features:** doc-id, last-access, access-velocity, embedding-recompute-cost
- **Action:** which docs to keep embed-cache-hot vs evict
- **Gain:** RAM-budget-bound: at constant RAM, hit-rate +20%
- **When viable:** when redb cache approaches budget limit

### 6. HW-aware kernel autotuner — NEW (Phase 3)
- **Algo:** XGBoost (offline tuning)
- **Features:** CPU model, AVX/NEON support, mem-bandwidth, L1/L2 cache size, parallel-cores
- **Output:** which simsimd kernel + thread-count for each hot-path
- **Gain:** +10-20% per platform vs static defaults
- **When viable:** after we have bench data on 3+ HW profiles

### 7. Slow-query alert with uncertainty — NEW (Phase 7)
- **Algo:** NGBoost (predicts both mean + variance)
- **Features:** query plan hash, recent latency time-series, brain-size growth
- **Action:** alert when actual p95 > predicted_p95 + 2σ → bisect changes
- **Gain:** catch perf regressions same-day instead of week-later
- **When viable:** ops monitoring track active

## Self-improving feedback loop architecture

```
[Synapse query log] (DuckDB ATTACH ~/.synapse/brain.db)
  ↓
[Nightly cron 02:00 UTC]
  → train all 7 models on prior 7-day window
  → write to ~/.synapse/models/{router,cache_warm,embed_selector,wp_spam,cache_priority,kernel_tuner,slowq}.cbm
  ↓
[Inference at request time] — load mmap, predict <100µs each
  ↓
[Shadow mode] for 1 week before promoting
  ↓
[A/B test] 50/50 with prior baseline
  ↓
[Promote] if measurable win, else extend shadow
  ↓
[Loop]
```

## Compound effect estimate

Phase 1+2+3 raw speed: 200-300× MySQL
+ Phase 2B router: +15-30% p95 on real traffic (ship in 30 days post-Phase-1)
+ Cache pre-warm (#2): +20% p50 on hot queries (ship in 45 days)
+ Embed-cache priority (#5): +20% hit-rate at constant RAM (ship in 60 days)
+ HW autotune (#6): +10-20% per platform (ship in 90 days)

**Compound 90-day ML uplift on top of raw: ~+50% p95, +35% hit-rate.**

Combined with Phase 1-7 raw speed gains: **300-400× MySQL gap closure** by Day 90.

## Implementation order recommendation

1. **Phase 1.5+:** wire query log → DuckDB (1 day, blocking #1, #2, #5, #7)
2. **Phase 2B implement** (3 days, biggest single gain)
3. **#2 cache pre-warm** (2 days, parallel to Phase 2)
4. **#5 cache priority** (2 days, after 2B)
5. **#6 kernel autotune** (5 days, after Phase 3)
6. **#3 embed selector** (1 week, requires data accumulation)
7. **#4 anti-spam** (post WP launch)
8. **#7 slowq alert** (post Synapse Cloud)

Total ML work: ~3 weeks of implementer effort over 90-day Phase span. All items are additive — no Phase blocked on these. ML layer is the **competitive flywheel** that makes Synapse smarter every night.

## Why this is the moat

Closed-source competitors (Pinecone, Algolia, Qdrant Cloud) can't ship per-customer-trained ML models. Synapse runs on YOUR data, on YOUR hardware. After 30 days of YOUR queries, your Synapse instance is optimized for YOUR workload. No SaaS can match that.

This is the Universal-Adapter pattern from the system-prompt rules, applied to the database itself.
