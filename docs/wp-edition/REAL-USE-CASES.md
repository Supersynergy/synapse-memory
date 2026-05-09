# synapsql — Real Use-Cases (refined target market)

After honest reality-check: **µs DB-savings disappear in network latency for end-user UX**.
**ABER**: capacity, cost, edge, AI, spike-survival = **REAL benefit**.

## Wo synapsql tatsächlich Sinn macht (€-relevant)

### 1. Hosting Provider / Agency mit 50+ Sites
**Pain**: Per-Site-Hosting kostet 5-50€/mo × 50 = 250-2500€/mo.

**Math**:
- MariaDB: 386µs autoload × 30 options × 1000 RPS = **38% CPU**
- synapsql: 0.74µs × 30 × 1000 = **0.07% CPU**
- 1 VPS hostet **8-10× mehr Sites** bei gleicher CPU-Load

**ROI**:
- Vorher: 10× $300 Kinsta = **$3000/mo**
- Nachher: 1× $80 VPS + synapsql = **$80/mo**
- **Saving: $2920/mo × 12 = $35k/Jahr**

**Markt**: 50.000+ EU-Hosting-Provider + Agenturen. Adressierbar 5-10%.

### 2. AI / RAG Backend
**Pain**: RAG-Anfrage = N× DB-vec-search inline. Jede ms zählt.

**Math**:
- 50 vec-search/Antwort × 5ms (Pinecone-class) = **250ms TTFT-Penalty**
- 50× 50µs (synapsql lokal MLX) = **2.5ms**
- **247ms Tokens-to-First-Token gespart** = User merkt Sekunden-Lag

**Wer kauft**: AI-Startups + RAG-SaaS (Glean, Vectara, etc).

### 3. Edge-Functions / Serverless
**Pain**: Cloudflare/Vercel Workers Cold-Start-Budget = **50ms total**.

**Math**:
- Postgres-Roundtrip via Hyperdrive: **5-15ms**
- Pinecone API: **50-200ms** (over Budget!)
- synapsql-edge embedded: **<1ms**

**Use**: Edge AI, Personalization-at-Edge, Geo-Routing-DB.

### 4. WooCommerce / Shopify Black-Friday Survival
**Pain**: Normal 100 RPS funktioniert, Spike 5000 RPS = MySQL crashes.

**Math**:
- MariaDB: timeouts ab **~1500 RPS** sustained
- synapsql batched: **>5000 RPS** stable (gemessen 792k ops/sec 8t)
- Differenz = **funktioniert vs $50k Verlust/30min Outage**

**Wer kauft**: Mid-tier WooCommerce-Stores ($1k-100k/Tag), Magento/Shopify-Plus-Tier.

### 5. WP-Admin Power-User Workflows
**Pain**: Editor-Save mit 200+ wp_options + 50 postmeta = **80-200ms DB-Zeit**.
Power-User speichert 100×/Tag → 8-20s wartezeit pro Tag pro user.

**Math**:
- MariaDB: 80ms/save × 100 saves = **8s/Tag DB-wait**
- synapsql cache: 0.5ms × 100 = **0.05s/Tag**
- 100 Editoren in einer SaaS = **800s = 13min/Tag eingespart**

**Wer kauft**: Multi-Author-Blogs (TechCrunch-class), CMS-SaaS.

### 6. Big-Catalog Product Filter (>50k Produkte)
**Pain**: WooCommerce/Magento JOIN posts × postmeta × terms = **600-2000ms p95**.

**Math**:
- MariaDB ohne Cover-Index: **800ms p95** filter-list
- synapsql columnar (P2 plan): **20-50ms p95**
- Geht von "Spinner sichtbar" zu "instant" — **echter UX-win**

**Wer kauft**: Mid-large Shops mit 10k+ SKUs.

### 7. Migration / ETL Windows
**Pain**: `wp-cli search-replace` über 100k posts = **18min downtime**.

**Math**:
- MariaDB single-row: **18min**
- synapsql batched: **2min**
- Migration-Window 18→2min = real Ops-Saving

**Wer kauft**: Enterprise-WP-Migrations-Agencies.

### 8. Backup-Window Collapse
**Pain**: xtrabackup 100GB = **4h Maintenance-Fenster**.

**Math**:
- MariaDB: 4h
- synapsql file snapshot: **30s** (just file copy + WAL checkpoint)

**Wer kauft**: Compliance-Schwere Sites (GDPR, SOC2).

## Wo synapsql **KEIN** Sinn macht

| Szenario | Warum |
|----------|-------|
| Hobby-Blog 100 visits/d | MariaDB ist fine, no spürbarer benefit |
| Single-User-Frontend-Pageload-UX | Network-Latenz dominiert (50-200ms RTT) |
| Static-Site mit Cloudflare CDN | DB-irrelevant, Edge-cache regiert |
| Low-traffic <1000 RPS | Capacity-Issue existiert nicht |
| MariaDB-Replication-User | synapsql kann (P3) noch keine Replication |
| Stored-Procedure-Heavy Legacy | synapsql kann nie volle PL/SQL |
| Single-Source-of-Truth $$$ | Production-replication essentiell, P5+ |

## Marketing-Tagline (ehrlich)

**Nicht**: *"100× faster WordPress"* (irreführend single-user-UX)

**Sondern**:
- *"Same hardware. 10× more sites."* (Hosting/Agency)
- *"Your AI backend at memory-speed."* (RAG/Edge)
- *"Survives Black Friday on $50 VPS."* (E-Com Spike)
- *"Save 13min/day per editor."* (CMS-SaaS)

## TAM-Schätzung (refined)

| Segment | TAM | ARR-Potenzial Year-3 |
|---------|----:|--------------------:|
| EU Hosting Providers | 50k | $50M |
| WooCommerce $1k+/Tag | 200k stores | $80M |
| AI/RAG Backends | 5k startups | $30M |
| Edge-Function-Devs | 20k | $20M |
| CMS-SaaS | 1k | $20M |
| Enterprise WP Migrations | 500 agencies | $10M |
| **Total** | — | **$210M** |

## Pricing aligned to real value

| Tier | Wer | $/mo | Why pays |
|------|-----|------|----------|
| **OSS Apache** | Hobbyist | 0 | Skip — nicht target |
| **Pro VPS** | 1-5 Sites | $25 | Spike-survival |
| **Agency** | 50-500 Sites | $249 | $2.5k Kinsta-saving |
| **Hosting Provider** | 1k+ Sites | $1500 | $30k SaaS-saving |
| **AI Backend** | RAG-SaaS | $499 | TTFT-saving = retention |
| **Enterprise** | E-com $$$ | $5k+ | Black-Friday-Insurance |

## Honest Marketing Disclaimer

> *"synapsql doesn't make your blog feel faster on a phone. It lets one server host 10× more blogs at the same speed your users see today. Capacity per CPU/€, not single-request UX."*

Das ist die ehrliche Verkaufs-Story. Friend's Kritik wird damit beantwortet, nicht ignoriert.
