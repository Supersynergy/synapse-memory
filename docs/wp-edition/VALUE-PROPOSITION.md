# synapsql — Value Proposition (Why entrepreneurs pay)

## End-in-mind: who pays + how much

### Tier 1: WooCommerce/Magento/Shopify Plus stores ($1k-100k/mo revenue)
**Pain**: 1s latency = 7% conversion drop (Amazon study). Black-Friday spikes crash Percona at 2k QPS.
**Pay**: $200-2000/mo for managed hosting (Kinsta, WP Engine), $99/yr for ObjectCache Pro plugin, $15-150/mo for caching CDN.
**synapsql sell**: 100× cached pageload + 7× checkout. Drop-in. Saves $1k-20k/mo hosting + recovers 2-7% conversions = $500-50k/mo revenue lift.

### Tier 2: High-traffic content sites (1M-100M monthly views)
**Pain**: Core Web Vitals bad → Google demotion → traffic loss. Hosting costs scale with QPS.
**Pay**: $500-5000/mo enterprise hosting (Pantheon, WP VIP $2k-20k/mo).
**synapsql sell**: 50-100× cached pages → green Core Web Vitals → SEO ranking lift. Replaces $5k/mo VIP hosting with $50/mo VPS + synapsql.

### Tier 3: Multi-tenant SaaS (Shopify-clones, white-label CMS)
**Pain**: Per-tenant DB isolation expensive. Postgres+RDS = $200-2000/mo per tenant.
**Pay**: Multi-tenant DB tier ($10-1000/tenant/mo).
**synapsql sell**: ATTACH per-tenant in single binary → 1 daemon serves 1000s tenants. 10-100× cheaper.

### Tier 4: Agencies + Consultancies (50-500 client sites)
**Pain**: Each client = own Kinsta plan = $2k-20k/mo aggregate.
**Pay**: $50-200/site/mo aggregated.
**synapsql sell**: 1× synapsql VPS ($50/mo) hosts 100 sites at 10× speed. Margin 95%+.

## Pricing model (3 tiers)

### OSS Community (Apache 2.0, free)
- Single binary, all features
- Self-host
- Forum support
- **Goal**: viral adoption, GitHub stars, market share

### Cloud Managed ($25-2500/mo)
| Plan | $/mo | Sites | Traffic | Vergleich |
|------|------|-------|---------|-----------|
| Starter | $25 | 1 | 50k visits | vs Kinsta $35 |
| Pro | $79 | 5 | 250k visits | vs Kinsta $115 |
| Business | $249 | 20 | 1M visits | vs Kinsta $345 |
| Scale | $799 | 100 | 5M visits | vs Kinsta $1500 |
| Enterprise | $2499 | unlimited | 25M visits | vs WP VIP $5k+ |

→ **30-50% cheaper than Kinsta/WP Engine** + **10-100× faster**.

### Enterprise License ($5k-50k/yr)
- Self-host with support
- Custom adapter dev (e.g. for proprietary CMS)
- SLA 99.99%
- 24/7 oncall
- Compliance audit (SOC2, ISO27001)
- Custom pricing $5k-50k/yr

### Plugin Marketplace ($99-999 lifetime)
- WP plugin: synapsql-wp ($99 lifetime)
- WooCommerce extension: synapsql-woo ($199 lifetime)
- Magento module: synapsql-magento ($299 lifetime)
- Shopify GraphQL middleware: $399/yr
- All-in-one bundle: $999 lifetime

## Why entrepreneurs pay

### 1. Direct revenue impact
- Amazon: 100ms = 1% sales loss
- Walmart: 1s slower = 2% conversion drop
- Pinterest: 40% slower → 15% sign-ups, 15% SEO traffic
- **Math**: $10k/mo Woo store with 7× checkout speedup = +$700-2000/mo revenue (3-7% conversion lift)

### 2. Cost reduction
- Replace $300/mo Kinsta with $50/mo VPS + synapsql
- **Math**: $250/mo savings × 12 = $3000/yr per site

### 3. SEO ranking
- Core Web Vitals → Google ranking factor since 2021
- Sub-200ms LCP → top-3 organic rankings → +20-50% organic traffic
- **Math**: 1M visit/mo blog = +200k visits = +$2k-20k/mo revenue

### 4. Black-Friday / spike survival
- 10× traffic spike = Percona crashes, lost sales
- **Math**: 1 day BF outage on $50k/day store = $50k loss
- synapsql 100k QPS sustained = no outage

### 5. Drop-in (no rewrite)
- Existing 100k+ LOC PHP codebase = no rewrite
- Just change `DB_HOST` env var
- **De-risk**: zero migration cost

## ROI calculator

For typical $10k/mo Woo store:

| Metric | Before (Percona+Kinsta) | After (synapsql self-host) | Delta |
|--------|------------------------|---------------------------|-------|
| Hosting | $345/mo | $50/mo | +$295/mo |
| Cache plugin | $99/yr | $0 | +$8/mo |
| Conversion rate | 1.8% | 1.95% (+8% rel) | +$800/mo |
| **Total ROI** | — | — | **+$1100/mo** |

**Payback**: synapsql Pro $79/mo → ROI 14× in month 1.

## Competitive moat

| | synapsql | Kinsta | LiteSpeed | ObjectCache Pro | wp-rocket |
|---|---|---|---|---|---|
| Apache OSS | ✅ | ❌ proprietary | ❌ | ❌ | ❌ |
| Drop-in (no code change) | ✅ | ✅ | ✅ | ✅ | ✅ |
| 100× cached | ✅ | 5× | 20× | 10× | 5× |
| WP-aware optimizer | ✅ | ❌ | ❌ | 🟡 | ❌ |
| Native vec+FTS+graph | ✅ | ❌ | ❌ | ❌ | ❌ |
| Self-host < $50/mo | ✅ | ❌ | 🟡 | ❌ | ❌ |
| Multi-platform (WP+Shopify+Mag+...) | ✅ | ❌ WP only | ❌ | ❌ WP | ❌ |
| Apache 2.0 EU | ✅ | ❌ | ❌ | ❌ | ❌ |

→ **Niemand hat alles**. synapsql first-mover in this combo.

## Go-to-market

### Phase 1: OSS launch (months 1-3)
- ProductHunt, HN, Reddit r/wordpress, r/woocommerce
- 100k stars target via "100× WordPress" benchmark virality
- Free tier captures 10k+ self-hosters

### Phase 2: Cloud beta (months 4-6)
- 100 paying customers @ $79 = $7.9k MRR
- Case studies: Black-Friday survival, $X savings

### Phase 3: Enterprise (months 7-12)
- 10 enterprise contracts @ $20k/yr = $200k ARR
- Plugin marketplace launches: targeting $50k/mo passive

### Year 1 target
- ARR $300k (cloud) + $200k (enterprise) + $100k (plugins) = **$600k ARR**
- 10k OSS users
- Top-3 GitHub Rust DB by stars

## Endgame (Year 2-3)

- $5M-20M ARR
- Acquisition target by Cloudflare, Fastly, Akamai, MongoDB, AWS
- Or independent SaaS scaling to $100M+ ARR

## Why now (May 2026 timing)

- WP 6.7 latest, plugin ecosystem mature
- WooCommerce HPOS adoption full
- Core Web Vitals as ranking factor → demand for speed
- Cloud DB prices rising (Pinecone, PlanetScale tier hikes)
- Apache OSS sentiment strong post-MongoDB SSPL drama
- M-series Mac dev parity → cheap dev environment
- Rust 1.95 stable, async maturity
