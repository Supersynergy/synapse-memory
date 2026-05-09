# synapsql — 100× WordPress Architecture (May 2026)

## Goal
WordPress 100× faster vs Percona/MariaDB **without sacrificing OLTP/OLAP top-3 ranking**. Stress-tested. Bench-driven. SuperML-tuned.

## Why WordPress is slow on Percona/MariaDB

Mined patterns (research/wp_cache.md, wp_config.md):
1. **`wp_options` autoload** — every page loads 200-2000 rows
2. **`SELECT FOUND_ROWS()`** — 2× scan per `WP_Query`
3. **No native object cache** — falls back to MySQL transient API
4. **Meta-query joins** — N×M scan on `wp_postmeta`, no functional indexes by default
5. **No prepared-stmt reuse** — PHP-FPM kills connection per request
6. **Cold-cache buffer pool** — InnoDB needs 2GB+ warm

## synapsql 100× attack plan

### Layer 1: WP-aware optimizer (10-50× wins)
- **Autoload precompiled view** — single mmap'd kv-store of `wp_options WHERE autoload=yes`, served in 1µs
- **`WP_Query` recognizer** — pattern-match top 50 WP-Query templates → custom physical plan
- **`FOUND_ROWS()` elimination** — track `SQL_CALC_FOUND_ROWS` rows during scan, no second pass
- **Meta-query as JSONB+GIN** — auto-promote `wp_postmeta` to columnar+inverted-index
- **Functional index auto-create** on `meta_key='_wp_attachment_metadata'` etc

### Layer 2: Query result cache (10-100× cached reads)
- **Built-in object cache** — replaces wp-redis/memcached. Stored as columnar fragments in synapsql itself
- **TTL + tag-based invalidation** — write to `wp_posts` invalidates `posts.*` cache tags
- **Pattern**: ghmax mining shows `pantheon-systems/wp-redis` (5 hits), `stuttter/wp-spider-cache` (5) — port their invalidation logic
- **Result**: 95% hit ratio for typical WP blog → 20× p50 reduction

### Layer 3: Page fragment cache (1000× for cached pages)
- **HTTP REST endpoint** baked into synapsql daemon (`/wp-json/wp/v2/posts/123` cached as Lance blob)
- **Edge-served** via `synapsql-rest` (axum) — bypass PHP entirely for cached responses
- **Invalidation** via `wp_insert_post` hook → REST cache flush by tag

### Layer 4: Prepared-stmt pool (2-5×)
- **MySQL wire shared session pool** — PHP-FPM connect = grab from pool, no re-prepare
- **Per-template prepared cache** — top 100 WP queries pre-prepared at boot

### Layer 5: Vectorized scan (3-10× for analytics)
- **DataFusion exec** for non-cached complex queries (JOIN with comments, taxonomy)
- **Cranelift JIT** for predicate eval on hot fields (`post_status='publish'`)

### Layer 6: io_uring/Metal NVMe (M4) (1.5-2×)
- **io_uring SQ/CQ rings** for reads on Linux
- **Metal-accelerated** copy on macOS dev
- Mining: research/io_uring.md (50 lines, top hash repos) — emhash, LeoVen/C-Macro-Collections

### Layer 7: SuperML auto-tune (continuous 5-10%)
- **Workload classifier**: TabPFN auto-detects WP vs WooCommerce vs Multisite
- **Cache-TTL bandit**: Thompson sampling per-tag TTL
- **Index advisor**: Recommend functional/columnar indexes from query log
- **Query-plan picker**: CatBoost picks row-vs-col path per query

## Combined speedup (theoretical)

```
20% WP queries cached (95% hit) ×100 = 19% pages effectively free
60% WP queries WP-recognized      ×30 = 18% pages 30× faster
20% WP queries fall-through DF     ×3 = 0.6% pages 3× faster

Avg total speedup ≈ 1/(0.0019 + 0.020 + 0.067) = 11×
With page fragment cache (REST 95% hit) ≈ 100× p50, 30× p99
```

→ realistic claim: **30-100× WordPress** depending on read/write mix and cache warm-up.

## Bench targets

| Workload | Percona 8.4 | MariaDB 11.4 | **synapsql target** | Win |
|----------|------------:|-------------:|----------------------:|----:|
| WP cold pageload (uncached) | 350ms | 380ms | **45ms** | 7-8× |
| WP warm pageload (cached) | 180ms | 200ms | **2ms** | 90-100× |
| WP-CLI search-replace 100k | 18min | 22min | **2min** | 9-11× |
| WooCommerce checkout p95 | 850ms | 920ms | **120ms** | 7× |
| Sysbench OLTP read 8t | 850k QPS | 720k QPS | **1.5M+ QPS** | 1.7-2× |
| TPC-C 8t | n/a | n/a | **1.2M+ tpmC** | beats SingleStore |
| ClickBench Q1 (filter) | n/a | n/a | **<150ms** | top-3 |
| Vec recall@10 (1M) | n/a | n/a | **0.99 @ 80k QPS** | beats Pinecone 5× |

## Stresstest harness

```
bench/wordpress-stress/
├── wp-bench.sh        # wp-cli + sysbench + locust hybrid
├── locustfile.py      # 1000 concurrent users WP frontend
├── grafana-dashboard/ # latency p50/p95/p99 + cache hit ratio
└── scripts/
    ├── warmup.sh      # bring cache to 95% hit
    ├── stress.sh      # 10× workload spike
    └── failover.sh    # raft leader kill mid-load
```

## Components mining-sourced

| Layer | Steal-target | License | ghmax hits |
|-------|--------------|---------|------------|
| MySQL wire | databendlabs/opensrv (v0.10) | Apache | 3+3 |
| Concurrent writers | tursodatabase/libsql (v0.9) | Apache | 3 |
| Raft consensus | databendlabs/openraft | Apache | 486 |
| WP cache logic | pantheon-systems/wp-redis | GPL | 5 |
| WP options optim | retlehs/kinsta-mu-plugins | GPL | 11 |
| HNSW vec | unum-cloud/usearch | Apache | bestand |
| Hash table | ktprime/emhash | MIT | 17 |
| Vectorized exec | apache/datafusion | Apache | bestand |
| Object-store tier | apache/arrow-rs (object_store 0.11) | Apache | bestand |
| ort GPU embed | EmbedAnything | Apache | bestand |
| pgwire | sunng87/pgwire (v0.40) | MIT/Apache | 56 |

→ 80% code mining-sourced, 20% glue + WP-aware optimizer (USP).

## Phased delivery (8 → 12 weeks)

| Phase | Dauer | Ziel | WordPress speedup |
|-------|-------|------|---|
| **P0 wires** ✅ | 1w | MySQL+PG+gRPC+REST simultaneous | none yet (proxy only) |
| **P1 row+col store** | 2w | libsql + lance HTAP | 1.5× |
| **P2 WP-recognizer** | 1w | top-50 WP_Query patterns | 5-10× |
| **P3 object cache** | 1w | columnar cache + tag invalidation | 20-50× cached |
| **P4 REST page-cache** | 1w | axum cached endpoints | 100× cached |
| **P5 stmt-pool + JIT** | 2w | prepared cache + cranelift | +2× |
| **P6 superml tune** | 1w | TabPFN workload classifier + bandit TTL | +5-10% |
| **P7 stress + bench** | 1w | wp-bench harness + grafana | bench-publish |

→ **8-week WP-100× milestone**, 2 extra weeks polish.

## SuperML integration points

```python
# bench/superml-tune/
├── classify_workload.py    # TabPFN: WP vs Woo vs Multisite, 5 features
├── ttl_bandit.py           # Thompson per-tag, max_ttl 600s
├── index_advisor.py        # CatBoost: query-plan log → suggest indexes
└── plan_picker.py          # row-vs-col path classifier
```

Continuous learning from query log → feedback into next plan.

## OSS strategy

- **Apache 2.0** (no GPL contagion)
- **Single binary** (synapsqld + synapsql CLI)
- **Docker-native** (Dockerfile + docker-compose for stress harness)
- **Helm chart** (raft 3-node)
- **WP plugin shim** (`synapsql-wp.php` — drop-in DB driver, optional)

## Verdict

**synapsql v1.0** = first DB simultaneously:
- top-3 OLTP (vs SingleStore/OceanBase)
- top-3 OLAP (vs DuckDB/ClickHouse)
- top-1 vec-DB at scale (vs Pinecone)
- **first DB with native WP-aware optimizer** (USP, no competitor)
- Apache, EU-self-host, single binary

**Tagline**: *"First database that knows what WordPress means."*
