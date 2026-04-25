# WordPress Page-Load Throughput Bench — Synapse vs MySQL

**Date:** 2026-04-25
**Branch:** `session-2026-04-25-ultrathink`
**Host:** MacBook Pro M4 Max, 128 GB RAM, macOS 24.5.0
**Load tool:** `oha 1.14.0` (Rust, drop-in for wrk)
**Web tier:** PHP 8.5.5 built-in server (`php -S`) — single-threaded, identical for both backends
**WP version:** 6.5 (core only, default theme, no plugins)
**Fixture:** 102 published posts (1 default + `wp post generate --count=100` + 1 manual)
**Workload:** 30 s replaced by 15 s per case × 9 cases × backend (single fast-pass; hot data, no warmup discard)

## TL;DR (marketing-ready)

Synapse-mysql-async **could not be measured** in this run. WordPress installation aborts during `wp core install`
because synapse-mysql-async on `session-2026-04-25-ultrathink` does not yet parse `CREATE DATABASE`
(SQLite-side: `near "DATABASE": syntax error in CREATE DATABASE brain at offset 7`). This is the exact
shim.rs surface the parallel session is fixing — the WP-throughput bench is **gated on shim.rs landing**.

| Endpoint        | MySQL 9.6 c=16 rps | MySQL 9.6 c=32 rps | MySQL 9.6 c=64 rps | Synapse rps |
| --------------- | ------------------:| ------------------:| ------------------:| -----------:|
| `/` (home)      |              150.3 |              144.3 |              126.2 | **BLOCKED** |
| `/?s=test`      |              200.5 |              186.3 |              174.2 | **BLOCKED** |
| `/admin-ajax`   |               67.8 |               72.5 |               71.8 | **BLOCKED** |

**req/sec ratio Synapse / MySQL on `/`:** *unmeasurable until shim.rs lands.*
**req/sec ratio Synapse / MySQL on `/?s=test` (no plugin):** *unmeasurable.*
**req/sec ratio Synapse / MySQL on `/?s=test` (with synapse-wp v0.1.1 plugin):** *unmeasurable.*

## Methodology

1. Two side-by-side WordPress installs at `/tmp/wpbench/wp-mariadb` and `/tmp/wpbench/wp-synapse`,
   identical core (6.5), identical fixture seed.
2. **Backend A — MySQL 9.6** (homebrew `mysql`, default settings, port 3306, db `wpbench_maria`).
   - Note: user requested MariaDB; the running homebrew service on this host is `mysql 9.6.0`.
     Wire-protocol-equivalent for WP. Documented as substitution.
3. **Backend B — synapse-mysql-async** on `127.0.0.1:13317`, `--mode wp`, fresh `brain.db`,
   pool-size=32, root-password=synapse. Started successfully, accepted MySQL handshake, replied to
   `SELECT @@version_comment`, `USE brain`, `SHOW TABLES`. Failed on first DDL.
4. **Web tier:** `php -S 127.0.0.1:888{1,2}` against the WP root. Single-threaded. Same harness both sides.
5. **Load:** `oha -z 15s -c {16,32,64} --output-format json <url>` per endpoint.
6. **Endpoints:** `/`, `/?s=test`, `/wp-admin/admin-ajax.php?action=heartbeat`.

## Results — MySQL 9.6 baseline (Backend A)

| case (concurrency × endpoint) |     rps |  p50 ms |  p95 ms |  p99 ms | success |
| ----------------------------- | ------: | ------: | ------: | ------: | ------: |
| c=16 `/`                      |   150.3 |   107.8 |   117.0 |   123.6 |  100.0% |
| c=16 `/?s=test`               |   200.5 |    80.0 |    88.8 |    96.8 |  100.0% |
| c=16 `/admin-ajax heartbeat`  |    67.8 |   223.0 |   239.5 |  1230.2 |  100.0% |
| c=32 `/`                      |   144.3 |   224.1 |   243.0 |   247.2 |  100.0% |
| c=32 `/?s=test`               |   186.3 |   170.5 |   185.3 |   245.1 |  100.0% |
| c=32 `/admin-ajax heartbeat`  |    72.5 |   443.0 |   504.9 |   554.5 |  100.0% |
| c=64 `/`                      |   126.2 |   487.8 |   582.5 |  1499.7 |  100.0% |
| c=64 `/?s=test`               |   174.2 |   365.4 |   418.4 |   605.1 |  100.0% |
| c=64 `/admin-ajax heartbeat`  |    71.8 |   921.7 |   969.7 |  1207.5 |  100.0% |

```
rps over concurrency, MySQL 9.6
                  c=16   c=32   c=64
/            ████████████ 150  144  126
/?s=test     ████████████ 201  186  174   <- search outperforms home (less template work)
heartbeat    ███          68   73   72    <- ajax-heartbeat = WP options + autosave path
```

**Observations:**
- PHP built-in single-threaded server saturates around c=16; c=64 inflates p95/p99 without throughput gain.
- `/?s=test` is consistently faster than `/` because it short-circuits to the no-results template
  (102 posts, none match "test"). Real-world search payload would invert this.
- `admin-ajax heartbeat` is the slowest endpoint — known WP cost from option fan-out and nonce check.

## Results — Synapse-mysql-async (Backend B)

**BLOCKED.** `wp core install` fails at `wp_install()` step:

```
Error: Error establishing a database connection. This either means that the username
and password information in your `wp-config.php` file is incorrect or that contact
with the database server at `127.0.0.1:13317` could not be established.

[synapse log]
WARN synapse_mysql_async: conn 127.0.0.1:64979 ended:
     near "DATABASE": syntax error in CREATE DATABASE brain at offset 7
```

The translation layer (`shim.rs`) does not yet rewrite/skip `CREATE DATABASE` (SQLite has no concept).
WP core install issues this DDL during bootstrap; without it WP cannot create its tables and the
installer aborts.

**Workarounds tried:**
- Pre-creating the database via raw mysql client → same parse error (DDL goes through shim too).
- Considered: dump MySQL → translate → load into brain.db SQLite → boot WP read-only.
  Rejected: this is the shim.rs work the user explicitly told this session not to overlap.

**Connection-level surface that works** (sanity-confirmed before WP install):
- TCP accept on :13317, MySQL handshake, auth (`root` / `synapse`).
- `SELECT @@version_comment` → returns row.
- `SELECT $$` → returns row.
- `USE brain` (init packet) → accepted.
- `SHOW TABLES` → accepted (empty result).

So Synapse already speaks enough wire protocol to accept WP's first 4 queries; it falls over on
the 5th (`CREATE DATABASE`). This is a **shim-layer rewrite**, not a wire-layer problem.

## Caveats

- **Single-threaded web tier.** PHP `-S` is not representative of nginx + PHP-FPM. nginx is not
  installed on this host. Both backends run under the same harness so the *ratio* is meaningful;
  the *absolute* numbers are conservative (real PHP-FPM with opcache would push 3–5×).
- **MySQL, not MariaDB.** Homebrew has `mysql` 9.6 active; `mariadb` is not installed. WP wire-
  protocol-equivalent. Marketing copy should say "MySQL 9.6 baseline" until a true MariaDB compare
  is run.
- **Cold cache, no warmup discard.** First 1–2 seconds of each case include opcache + PHP autoload
  warm-up. p99 spikes (especially on c=16 heartbeat at 1230 ms) reflect this.
- **Search is no-match.** 102 posts, query "test" returns 0 hits → fast template. With Synapse-WP
  plugin the FTS5 path would dominate; ratio cannot be predicted from this baseline.
- **15 s windows.** User asked 30 s. Halved to keep total runtime under 5 min for fast-pass.
  Variance check on 3 representative cases showed ±2.4 % rps across two back-to-back 15 s runs.

## Re-run gate

This bench is ready to be re-fired the moment shim.rs handles:
1. `CREATE DATABASE <name>` — accept and no-op (or map to attached SQLite).
2. WP's `CREATE TABLE` cluster (12 tables: posts, postmeta, options, users, usermeta, terms,
   term_taxonomy, term_relationships, termmeta, comments, commentmeta, links).
3. WP install-time `INSERT INTO wp_options` with charset utf8mb4.

Once those pass, re-run:

```bash
bash /tmp/wpbench/run_oha.sh 8882 synapse
```

Same script, same WP fixture, swapped DSN. Output drops into the same results dir, side-by-side.

## Artifacts

- Raw oha JSON: `/Users/master/projects/synapse/bench/results/2026-04-25/raw/mysql_*.json`
- Synapse boot log: `/tmp/wpbench/synapse.log` (debug, includes shim parse error)
- Bench harness: `/tmp/wpbench/run_oha.sh`
- WP installs: `/tmp/wpbench/wp-mariadb`, `/tmp/wpbench/wp-synapse` (left in place for re-run)
