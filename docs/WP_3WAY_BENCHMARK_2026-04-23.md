# WP 3-Way DB Benchmark — 2026-04-23

**Backends**: MySQL 8.0 · Percona Server 8.0 · Synapse-MySQL  
**Stack**: WordPress 6.x (php8.2-apache), Docker compose, host macOS M4 Max 128GB  
**Method**: 10 real WP use-cases, median of 5 runs per HTTP UC, single-shot for bulk DB ops  
**Branch**: `wp-bench-3way`

---

## Results

| Use Case | MySQL 8.0 (ms) | Percona 8.0 (ms) | Synapse-MySQL |
|----------|---------------|-----------------|---------------|
| UC1 Cold homepage TTFB | 76 | 62 | BLOCKED |
| UC2 Warm homepage | 73 | 79 | BLOCKED |
| UC3 REST posts list (50) | 90 | 48 | BLOCKED |
| UC4 Single post fetch | 63 | 70 | BLOCKED |
| UC5 Search `?s=lorem` | 75 | **486** | BLOCKED |
| UC6 wp_options bulk SELECT | <1 | <1 | BLOCKED |
| UC7 Insert post | 9 | 1 | BLOCKED |
| UC8 Insert 100 comments | 88 total | 183 total | BLOCKED |
| UC9 Update post meta 100× | 79 total | 111 total | BLOCKED |
| UC10 Concurrent reads (ab -c8 -n200) | **105 p50** | INVALID† | BLOCKED |

†UC10 Percona: DB OOM-killed during ab run; 500-errors measured (not real throughput)

---

## Synapse-MySQL: BLOCKED at step 1

WP fires `SET sql_mode = ...` immediately after connect (in `wpdb::set_sql_mode()`).  
Synapse-MySQL returns a non-standard response for this statement — `mysqli_fetch_array()` receives `bool(true)` instead of a `mysqli_result`, causing a PHP fatal error before any page can render.

**Root cause**: Synapse-MySQL is a SQLite-backed MySQL-wire-protocol server. It does not implement `SET sql_mode` in a mysqlnd-compatible way. All 10 UCs are blocked.

**Workaround that would enable testing**: Patch `wp-includes/class-wpdb.php` to skip `set_sql_mode()` when connecting, or add `SET sql_mode` stub support to synapse-mysql returning an empty OK result.

---

## Top-3 Surprises

1. **Percona search 6.5× slower than MySQL8 on `?s=lorem`** (486ms vs 75ms).  
   WP uses `LIKE '%lorem%'` with no fulltext index by default. Percona 8.0's optimizer chose a different execution plan — likely full table scan with stricter InnoDB page prefetch. Reproduces across runs.

2. **Percona OOM-crashes under sustained load** (exit 137 × 3 during test run).  
   Percona 8.0 has a larger base memory footprint than MySQL 8.0. On a Docker-constrained host running 3 MySQL instances simultaneously, Percona was repeatedly killed by OOM. UC10 ab result is therefore invalid for Percona.

3. **Percona REST API 2× faster than MySQL8** (48ms vs 90ms for `wp-json/wp/v2/posts`).  
   The REST endpoint serializes post data with multiple sub-queries. Percona's thread-pool and InnoDB buffer pool tuning appear to benefit this access pattern significantly when memory is not under pressure.

---

## Honest Limits

| Limit | Impact |
|-------|--------|
| Synapse-MySQL BLOCKED at WP connect | 0/10 UCs measurable; root cause documented |
| Percona OOM-killed 3× during run | UC10 invalid; UC8/UC9 measured under memory pressure |
| UC6 sub-1ms for both | Resolution too coarse to differentiate — need 1000-row option tables |
| Single-host Docker | All 3 DBs share CPU/RAM/disk; not isolated benchmark |
| WP object cache not disabled for UC2+ | `wp_options` transients only cleared for UC1; WP PHP cache still warm |
| ab on macOS fails with `localhost` | Must use `127.0.0.1`; IPv6 socket issue in macOS ab |

---

## Files

| File | Purpose |
|------|---------|
| `bench/wp-3way/docker-compose.yml` | 3-backend compose (MySQL:13306, Percona:13307, MySQL/synapse-upstream:13308; WP :18081-18083) |
| `bench/wp-3way/setup.sh` | WP-CLI install + 20 posts per backend |
| `bench/wp-3way/run.sh` | 10 UCs, parallel per backend, appends results.csv |
| `bench/wp-3way/results.csv` | Raw results with notes |
| `docs/WP_3WAY_BENCHMARK_2026-04-23.md` | This document |

---

## Recommendation

To make Synapse-MySQL measurable in WP context:

```sql
-- Stub needed in synapse-mysql query handler:
SET sql_mode = ...  →  return empty OK result (not SELECT result)
SET NAMES ...       →  return empty OK result
SET character_set_* →  return empty OK result
```

Once that stub is in place, re-run this benchmark — Synapse-MySQL's SQLite backend will likely show drastically different characteristics on write-heavy UCs (UC7-9) and read-heavy UCs (UC3, UC10).
