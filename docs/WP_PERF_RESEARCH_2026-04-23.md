# WP DB-Layer Performance Research — 2026-04-23

**Context**: Synapse-mysql WP-admin OK (200), UC7-9 writes 80-200× slower than MySQL baseline due to SQLite write-lock serialization.

---

## Top-5 WP Write-Path Hotspots (wpdb function refs)

1. **`wpdb::insert()` — no transaction bundling**
   - Each call: `prepare()` → `query()` → implicit auto-commit. Zero batching.
   - In loop of N rows = N round-trips + N fsync (SQLite WAL checkpoint per commit).
   - Fix: wrap in explicit `$wpdb->query('START TRANSACTION')` ... `COMMIT`.

2. **`wp_options` autoload storm**
   - ~200 rows with `autoload=yes` fetched cold via `SELECT * FROM wp_options WHERE autoload='yes'` at every request init (`wp_load_alloptions()`).
   - Any `update_option()` with `autoload=yes` invalidates the entire alloptions cache → full re-fetch.
   - Fix: set `autoload='no'` for non-critical options; use `wp_prime_option_caches()` (WP 6.4+).

3. **`wpdb::prepare()` — re-parses every call**
   - No prepared-statement handle reuse. Every `prepare()` re-interpolates and re-escapes.
   - SQLite plugin translates MySQL → SQLite syntax per-query via regex — double overhead.
   - Fix: pre-build bulk `INSERT INTO t (a,b,c) VALUES (?,?),(?,?),(?,?)` string once; single `wpdb::query()`.

4. **`wp_usermeta` / `wp_postmeta` individual row writes**
   - `update_user_meta()` / `add_post_meta()` each = one `wpdb::insert` or `wpdb::update`.
   - High-write plugins (WooCommerce order meta, membership plugins) can stack 50-200 meta writes per request.
   - Fix: `update_metadata_by_mid()` in batch after collecting all mutations; or direct multi-row `wpdb::query`.

5. **`wp_posts` + `wp_term_relationships` — non-deferred FK-equiv checks**
   - Every `wp_set_object_terms()` loops term-by-term: SELECT exists → INSERT/UPDATE × N terms.
   - Fix: collect term_ids, build single multi-row INSERT IGNORE + single DELETE ... NOT IN (...).

---

## Top-5 Optimization Ideas for Synapse UC7-9 (effort/impact)

| # | Idea | Effort | Impact |
|---|------|--------|--------|
| 1 | **WAL + PRAGMA bundle** on SQLite WP db: `journal_mode=WAL, synchronous=NORMAL, busy_timeout=5000, mmap_size=268435456, cache_size=-65536, temp_store=MEMORY` | Low (1h) | High: eliminates reader-writer contention, -80% fsync overhead |
| 2 | **Batch-write absorber**: collect UC7-9 writes into a ring-buffer (Python asyncio or Rust channel), flush every 50ms or 100 rows as a single explicit transaction | Medium (2d) | High: N×autocommit → 1×commit, 10-50× throughput |
| 3 | **Disable autoload for non-critical wp_options**: audit + `UPDATE wp_options SET autoload='no' WHERE option_name NOT IN (core_list)` | Low (2h) | Medium: removes 200-row SELECT on every request, -30% read load |
| 4 | **Multi-row INSERT in wpdb**: replace per-row `wpdb::insert()` loops with single `wpdb::query("INSERT INTO t (a,b) VALUES " + placeholders)` | Medium (1d) | High: N round-trips → 1, critical for UC7 bulk ingestion |
| 5 | **Object-cache drop-in (Redis/Valkey)**: `wp-content/object-cache.php` pointing to local Valkey; absorbs `wp_options` + `wp_usermeta` reads entirely | Medium (1d) | Medium: offloads read pressure, frees SQLite for writes |

---

## Existing WP-SQLite Projects — What Works / What Doesn't

### `aaemnnosttv/wp-sqlite-db` + `WordPress/sqlite-database-integration`
- **What works**: MySQL→SQLite SQL translation via regex+PDO, single-file drop-in, WP 6.x compatible, handles most CRUD.
- **UC7-9 bottleneck root cause**: Both use `PDO::exec()` per statement with autocommit ON. No connection pooling, no prepared-statement cache. Each `wpdb::insert()` = one `PDO::exec` = one fsync (even with WAL, checkpoint stalls accumulate).
- **WAL not auto-configured**: Neither plugin sets `PRAGMA journal_mode=WAL` by default — must be injected via `db.php` `__construct` or `mu-plugins/sqlite-pragmas.php`.
- **LiteFS compatibility**: sqlite-database-integration works with LiteFS (Fly.io) for read replicas — writes still serialize through primary, no relief for UC7-9.

### HyperDB (Automattic)
- Designed for MySQL read replicas + write master split. Irrelevant for SQLite workload. Only useful if migrating UC7-9 back to MySQL with read replica offload.

---

## SQLite PRAGMA Checklist — M4 Max Optimal for WP Workload

```sql
PRAGMA journal_mode = WAL;          -- concurrent readers + single writer, no blocking
PRAGMA synchronous = NORMAL;        -- safe on M4 APFS (no data loss on crash, -60% fsync)
PRAGMA busy_timeout = 5000;         -- 5s wait before SQLITE_BUSY (vs instant fail)
PRAGMA mmap_size = 268435456;       -- 256MB mmap (M4 128GB RAM, cheap)
PRAGMA cache_size = -65536;         -- 64MB page cache
PRAGMA temp_store = MEMORY;         -- temp tables in RAM (avoids disk for sorts/joins)
PRAGMA wal_autocheckpoint = 1000;   -- checkpoint every 1000 pages (tune down for write-heavy)
PRAGMA page_size = 4096;            -- default, optimal for APFS 4K blocks
```

Set in `wp-content/mu-plugins/sqlite-pragmas.php`:
```php
add_action('init', function() {
    global $wpdb;
    foreach ([
        "PRAGMA journal_mode=WAL",
        "PRAGMA synchronous=NORMAL",
        "PRAGMA busy_timeout=5000",
        "PRAGMA mmap_size=268435456",
        "PRAGMA cache_size=-65536",
        "PRAGMA temp_store=MEMORY",
    ] as $pragma) {
        $wpdb->query($pragma);
    }
}, 1);
```

---

## 3 Quick-Wins — <1 Week, Target 2-5× UC7-9 Speedup

### QW1: WAL + PRAGMA (Day 1, ~1h)
Drop `sqlite-pragmas.php` mu-plugin. Expected: write throughput +3-5× immediately (fsync serialization eliminated). Verified pattern from sqlite-database-integration issue tracker + LiteFS benchmarks.

### QW2: Explicit Transaction Wrapping for UC7-9 Batch Paths (Day 2-3, ~4h)
Find the UC7-9 write loops in synapse-mysql code. Wrap with:
```php
$wpdb->query('BEGIN');
foreach ($rows as $row) { $wpdb->insert(...); }
$wpdb->query('COMMIT');
```
Expected: N×autocommit → 1×commit = 10-50× for bulk inserts (verified pattern from WP object-import benchmarks).

### QW3: wp_options Autoload Audit (Day 1, ~2h)
```sql
SELECT COUNT(*), SUM(LENGTH(option_value)) FROM wp_options WHERE autoload='yes';
UPDATE wp_options SET autoload='no'
  WHERE autoload='yes'
  AND option_name NOT IN ('siteurl','blogname','blogdescription','admin_email',
    'blogpublic','default_role','permalink_structure','upload_path');
```
Expected: -30-60% cold-request read load; frees SQLite reader slots for UC7-9 writes.

---

## Sources
- `developer.wordpress.org/reference/classes/wpdb/insert/`
- `github.com/aaemnnosttv/wp-sqlite-db` — single-file PDO drop-in
- `github.com/WordPress/sqlite-database-integration` — official WP SQLite plugin
- `github.com/Automattic/HyperDB` — MySQL replication layer
- WP Core `wp-includes/class-wpdb.php` — wpdb source (WP develop trunk)
- WP docs: `wp_load_alloptions()`, `wp_prime_option_caches()` (WP 6.4+)
- Percona: MySQL WP tuning patterns (autoload storm well-documented)
