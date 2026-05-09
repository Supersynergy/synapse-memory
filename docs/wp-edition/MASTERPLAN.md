# synapsql — Masterplan (May 2026)

## Vision

**Drop-in DB layer** that powers every major CMS/e-com platform 10-100× faster than Percona/MariaDB. Apache 2.0, EU self-host.

```
┌─────────────────────────────────────────────────────────┐
│  CMS/E-Com (existing apps, zero code change)            │
│  WordPress · Shopify · Magento · Drupal · Ghost · Strapi│
│  WooCommerce · BigCommerce · PrestaShop · Joomla        │
└──────────────────────┬──────────────────────────────────┘
                       │ MySQL/PG/REST/GraphQL wire
                       ↓
            ┌──────────────────────┐
            │   synapsqld daemon   │
            │   (single binary)    │
            └──────────┬───────────┘
                       ↓
            ┌─────────────────────────────────────┐
            │  Platform-aware optimizers          │
            │  WP · Shopify · Magento · Drupal    │
            └──────────────┬──────────────────────┘
                           ↓
            ┌─────────────────────────────────────┐
            │  HTAP core (libsql + lance + DataFusion JIT)│
            │  Vec + FTS5 + Graph + ColBERT       │
            │  io_uring + Metal + raft + S3-tier  │
            └─────────────────────────────────────┘
```

## Platform support matrix

| Platform | Wire used | Adapter | Speedup target | LOC |
|----------|-----------|---------|----------------|-----|
| **WordPress 6.7+** | MySQL | `wp-aware-optimizer` | 100× cached, 7-10× cold | ~800 |
| **WooCommerce 9+** | MySQL | `woo-cart-optimizer` | 7× checkout, 30× catalog | ~600 |
| **Shopify** (self-host fork via wedding) | GraphQL | `shopify-storefront-cache` | 50× catalog reads | ~500 |
| **Magento 2.4+** | MySQL | `magento-eav-flatten` | 20× product page | ~700 |
| **Drupal 11+** | MySQL/PG | `drupal-render-cache` | 30× page | ~400 |
| **Ghost 5+** | MySQL | `ghost-content-cache` | 50× post-list | ~300 |
| **Strapi 5+** | PG | `strapi-graphql-cache` | 30× content | ~400 |
| **PrestaShop** | MySQL | `prestashop-product-feed` | 25× catalog | ~400 |
| **BigCommerce** (Stencil API) | REST | `bc-api-cache` | edge cache | ~300 |
| **Generic** | MySQL/PG/REST | none — wire passthrough | 1.5-2× from JIT | 0 |

→ **Drop-in for 10 platforms**, ~5000 LOC adapter total. 80% shared core.

## Adapter pattern (reused for all)

```rust
trait PlatformOptimizer: Send + Sync {
    fn matches(&self, sql: &str) -> Option<PlanHint>;
    async fn execute(&self, hint: PlanHint, store: &dyn Store) -> Result<QueryResult>;
}

// Each platform = ~1 crate, ~500 LOC pattern matching + plan-rewrite
struct WpOptimizer;
struct WooOptimizer;
struct ShopifyOptimizer;  // GraphQL adapter, not SQL
// ...
```

Routing: regex/AST match top-50 query templates per platform → custom physical plan.

## Drop-in install

```bash
# 1-line install
curl -sSL synapsql.io/install | sh

# Replace MySQL endpoint:
# Before: DB_HOST=mysql.example.com:3306
# After:  DB_HOST=127.0.0.1:3306  # synapsqld

# WordPress wp-config.php — zero change
# Magento env.php — zero change
# Shopify — install GraphQL middleware plugin (1 file)
```

## Phased delivery

| Phase | Dauer | Ziel | Stress-test gate |
|-------|-------|------|------------------|
| **P0 wires** ✅ | 1w | MySQL+PG bound, EchoStore | daemon binds clean |
| **P1 row+col** | 2w | libsql + lance HTAP | TPC-C 700k QPS |
| **P2 WP optim** | 1w | top-50 WP_Query patterns | wp-bench 10× cold, 50× warm |
| **P3 object cache** | 1w | columnar cache + tag invalidation | 95% hit ratio @ 1k RPS |
| **P4 REST page-cache** | 1w | axum cached endpoints | Apache-bench 100× warm |
| **P5 Woo + Magento** | 2w | EAV flatten + checkout opt | WooStress 7× checkout |
| **P6 Shopify + Strapi** | 1w | GraphQL cache adapter | gql-bench 50× |
| **P7 Drupal + Ghost** | 1w | render-cache adapter | drush-bench 30× |
| **P8 raft + tier** | 1w | openraft + S3 cold | failover bench p99<50ms |
| **P9 superml tune** | 1w | TabPFN workload class | adaptive bandit gain 5-10% |
| **P10 stress + ship** | 2w | locust harness, 10k concurrent users | sustained 24h no-OOM |

→ **13 weeks total** P0→Ship. 11 weeks dev + 2 weeks polish/security.

## Continuous bench loop

`bench/` runs every commit via CI:

```
bench-runner (every PR + nightly)
├── tpc-c             # OLTP gegen Percona/MariaDB/OB
├── tpc-h             # OLAP gegen DuckDB/CH
├── clickbench        # OLAP standard
├── ann-bench         # Sift/GIST recall@10
├── ycsb              # KV + OLTP mix
├── wp-bench          # wp-cli + locust 1000-user
├── woo-stress        # WooCommerce checkout flood
├── shopify-gql       # GraphQL cart/checkout
├── magento-perf      # Magento perf-toolkit
└── failover          # raft leader-kill mid-load
```

Each bench → grafana dashboard, regression-gate <5% slowdown blocks PR.

## Mining strategy (ghmax × 50/phase)

Per phase: 50 ghmax queries parallel → top-3 repos pro cluster → batch-md-rs extract → smollm2 digest → synx put → SPEC.md → 7-worktree fanout → Sonnet impl → Opus arch-review → bandit-merge.

## SuperML continuous learning

```python
# Every query → log → nightly retrain
1. Workload classifier (TabPFN): WP vs Woo vs Shopify vs Magento vs ad-hoc
2. Cache TTL bandit (Thompson): per-tag TTL optimization
3. Index advisor (CatBoost): top-N missing indexes from query log
4. Plan picker (LightGBM): row-vs-col-vs-cache path per query
5. Anti-ban detector (XGBoost): bot scrapers vs real users (cache vs no-cache)
```

Bandit-feedback into next-day query routing → 5-10% continuous improvement.

## Bench targets after P10

| Workload | Konkurrenz #1 | synapsql target | Win |
|----------|---------------|-----------------|-----|
| WP cold p50 | Percona 350ms | **45ms** | 8× |
| WP cached p50 | Percona+wp-rocket 80ms | **2ms** | 40× |
| WP cached p99 | Percona+rocket 200ms | **8ms** | 25× |
| WooCommerce checkout p95 | Percona 850ms | **120ms** | 7× |
| Shopify cart-add p95 | LiteSpeed proxy 180ms | **15ms** | 12× |
| Magento product-page p50 | Percona 280ms | **22ms** | 13× |
| Sysbench OLTP 8t | SingleStore 1.5M QPS | **1.6M+ QPS** | match |
| TPC-H 22q | DuckDB 120s | **<55s** | 2× |
| Vec recall@10 (1M) | Pinecone 10ms | **<2ms @ 0.99** | 5× |

## Cost-of-build

- ghmax 500q (10× phases) = 0€
- 50× Sonnet impl = ~25€
- 10× Opus arch = ~10€
- M4 Max bench-infra = 0€
- **Total P0-P10: ~35€** für SOTA #1 in 18-22 of 20 categories + drop-in for 10 platforms
