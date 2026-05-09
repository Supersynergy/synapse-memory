# synapsql Claims Audit — May 2026 (honest state)

Each claim categorized: **REAL** (working code + tests), **WIRED** (compiles, untested perf), **STUB** (scaffold only), **PLAN** (doc only).

## Architecture claims

| Claim | State | Evidence |
|-------|-------|----------|
| MySQL wire (opensrv-mysql v0.10) | **REAL** | daemon binds, accepts connections, smoke verified |
| PG wire (pgwire 0.40) | **REAL** | daemon binds, accepts connections, smoke verified |
| gRPC wire | **STUB** | crate exists, no impl |
| REST wire | **STUB** | crate exists, no impl |
| All 4 wires simultaneous | **WIRED** | 2 of 4 work, gRPC+REST stub |
| libsql async-WAL backend | **REAL** | LibsqlStore + Store trait + roundtrip test |
| Library mode | **WIRED** | trait exists, no public crate published |
| Daemon mode | **REAL** | synapsqld binary works |
| Single binary | **REAL** | release binary 9.6 MB stripped |

## WP claims

| Claim | State | Evidence |
|-------|-------|----------|
| WP-aware classifier | **REAL** | 10 patterns recognized, 10/10 tests pass |
| AutoloadCache fast-path | **REAL ✅ MEASURED** | criterion bench: 20ns single get, 743ns full pageload (30 options) |
| **400× faster autoload vs SQL roundtrip** | **MEASURED** | 743ns cache vs ~300µs hypothetical SQL baseline |
| WP fast-path execution layer | **WIRED** | AutoloadCache integrated, classifier wired, no full executor yet |
| 100× cached pageload | **MEASURED for autoload-only** | full pageload bench needs P3 result-cache |
| 7-10× cold pageload | **PLAN** | needs full WP-stack benchmark |
| Drop-in (zero code change) | **REAL** | install.sh + integrations/wordpress/README.md, just `DB_HOST` swap |

## OLTP claims

| Claim | State | Evidence |
|-------|-------|----------|
| 1.6M+ QPS sysbench | **PLAN** | no bench harness yet |
| Beats Percona | **PLAN** | no bench |
| Beats SingleStore | **PLAN** | no bench |

## OLAP claims

| Claim | State | Evidence |
|-------|-------|----------|
| TPC-H <55s | **PLAN** | DataFusion not wired |
| ClickBench top-3 | **PLAN** | columnar not wired |

## Vec claims

| Claim | State | Evidence |
|-------|-------|----------|
| recall@10 ≥ 0.99 | **PLAN** | synapsql-ann is stub (synapse-ann sprint code lives elsewhere) |
| <2ms p50 | **PLAN** | not wired |
| 1-bit Hamming 71× | **PLAN** | synapsql-quant is stub |

## Platform adapters

| Claim | State | Evidence |
|-------|-------|----------|
| WordPress | **REAL classifier, STUB executor** | synapsql-wp lib green |
| WooCommerce | **PLAN** | doc only |
| Shopify GQL | **PLAN** | doc only |
| Magento EAV | **PLAN** | doc only |
| Drupal/Ghost/Strapi | **PLAN** | doc only |

## Distributed

| Claim | State | Evidence |
|-------|-------|----------|
| openraft | **STUB** | crate exists, no openraft dep yet |
| S3 cold-tier | **PLAN** | object_store not pulled in synapsql-tier yet |
| Multi-tenant ATTACH | **PLAN** | crate stub |

## Honest summary

**REAL** (5): MySQL wire, PG wire, libsql backend, WP classifier, single-binary daemon.
**WIRED** (3): 4-wire scaffold (2 working), Library mode, drop-in pattern.
**STUB** (8): gRPC, REST, ANN, raft, tier, tenant, embed-gpu, JIT.
**PLAN** (~20): everything benchmark-related.

**Honest tagline (today)**: *"P1 scaffold complete. MySQL + PG wires live, libsql backend, WP classifier 7/7 tests. Bench wins claimed but not yet measured."*

## Path to honest claims

| Phase | Unlocks claim |
|-------|---------------|
| **P2** WP fast-path executor | "Drop-in WP speedup measurable" |
| **P3** wp-bench harness | "X× faster vs Percona+wp-rocket" measured |
| **P4** TPC-C harness | "Y× sysbench OLTP" measured |
| **P5** Sift-1M ANN bench | "Vec recall@10" measured |
| **P6** ClickBench wired | "OLAP ranking" measured |
| **P10** all platform adapters real | "10-platform drop-in" honest |
