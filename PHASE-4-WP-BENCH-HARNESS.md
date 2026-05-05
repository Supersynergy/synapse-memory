# PHASE-4: WordPress Benchmark Harness

Date: 2026-04-25 · Owner: Team Gamma γ1/γ2/γ3
Source: subagent ad08cb8f1f84cff82 design output

## Goal
Prove Synapse 100× faster than vanilla WP+MariaDB on shared hosting (Hetzner CX11 simulation: 1 vCPU, 2GB RAM).

## 5 Bench Scenarios

| # | Scenario | MySQL | Synapse target | Gain |
|---|---|---:|---:|---:|
| 1 | Home (1k posts, widgets) | 280ms | **<50ms** | 5.6× |
| 2 | Single post + related | 320ms | **<80ms** | 4× |
| 3 | Search "rust web framework" | **1500ms** | **<8ms** | **187×** ⭐ |
| 4 | Admin posts.php | 600ms | **<100ms** | 6× |
| 5 | WooCommerce shop archive | 480ms | **<45ms** | 10.7× |

**DoD:** ≥6/8 metrics with ≥4× gain.

## Critical Bottleneck — wp_options autoload
- 300 rows × ~200 bytes = 60KB serialized blob
- Loaded on EVERY request (non-configurable)
- MySQL: ~12ms deserialization
- **Synapse fix:** dedicated cache path + zstd-3 compression → **0.5ms (24× faster)**

## Architecture

```
docker-compose.yml
├── wp-vanilla:8080    (WP + MariaDB 11)
├── wp-synapse:8081    (WP + synapse-mysql-async)
├── mariadb:3306
└── synapsed:3307
```
Both containers limited to **1 CPU + 1GB RAM** (shared host emulation).

## Tooling
- **k6** load harness with 5 scenarios, 30s each, constant-arrival-rate
- Per scenario: `__ENV.TARGET_URL` switches between vanilla/synapse
- Output JSON → parser → comparison table

## Seed data (init-1k-posts.sql)
- 300 wp_options (autoload=yes)
- 1000 wp_posts (publish, post)
- 5000 wp_postmeta (5 keys/post)
- 50 wp_terms, 3000 wp_term_relationships
- 10 wp_users, 500 wp_comments

## Scenario 3 deep dive (the killer demo)
Vanilla WP search:
```sql
SELECT * FROM wp_posts
WHERE (post_title LIKE '%rust%' OR post_content LIKE '%rust%')
AND (post_title LIKE '%web%' OR post_content LIKE '%web%')
-- Cartesian product → 1500ms
```
Synapse-WP:
```sql
SELECT * FROM wp_posts
WHERE synapse_match(post_title || post_content, 'rust web framework') > 0.7
-- HNSW vec → 8ms
```
**Revised: ~50× from FTS5 alone (10k+ posts), +3–5× from Synapse semantic layer in-process.**
Headline "187×" was conflated — see bench/results/2026-05-05/wp_bench_fair_decomp.md for decomposition.
Honest claim: "up to 100–250× over unindexed LIKE at 50k posts; FTS5 break-even ~5k rows; at 1k posts vanilla LIKE wins."

## Implementation Tasks
| Task | Owner | Est |
|---|---|---:|
| docker-compose + seed SQL | γ2 | 4h |
| k6 harness script | γ3 | 3h |
| synapse-mysql wire (Phase 1 dependency) | γ1 | 6h |
| WP+Synapse plugin integration | γ2 | 2h |
| Result parser + dashboard | γ3 | 2h |
| GitHub Actions CI | γ2 | 2h |

**Total:** ~14h work · target Day 49 of Phase 4

## Verdict Update — 2026-05-05

**Status: GO** (was: PAUSE pending fair decomposition)

Decomposition complete. Honest numbers do not kill the demo — they make it reproducible:
- 50× from FTS5 is real and measurable above 10k posts
- 3–5× Synapse semantic on top is real (in-process, no CLI IPC)
- Combined ~150–250× at 50k posts is defensible with disclosed methodology
- Drop "187×" headline; use "up to 100×" with footnote linking to fair-decomp doc

See: bench/results/2026-05-05/wp_bench_fair_decomp.md
