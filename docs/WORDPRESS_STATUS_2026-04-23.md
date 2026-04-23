# Synapse — WordPress Integration Status, 2026-04-23

## Scope

`crates/synapse-mysql` (640 LoC) + `wordpress-test/` containerized test rig + the upstream `sqlite-database-integration-2.2.23` plugin. All three are in-tree on `main`.

## Build status

| Component | Status | Note |
|---|---|---|
| `cargo build --release -p synapse-mysql` | **green** (19.6 s) | warns on dead `Acl::User` field |
| `cargo test --release -p synapse-mysql` | **0 tests pass / 0 fail** | no `#[test]` written |
| `target/release/synapse-mysql` binary | **present** (4.4 MB) | |

## Live test (containers running this session)

`docker ps` shows **5 running** containers (3 hours uptime):

| Container | Port | Backend |
|---|---:|---|
| `wordpress-test-wordpress-mysql-1` | 8083 | real MySQL 8.0 |
| `wordpress-test-wordpress-sqlite-1` | 8082 | upstream SQLite plugin (control) |
| `wordpress-test-wordpress-synapse-1` | **8084** | **Synapse-mysql wire** |
| `wordpress-test-mysql-1` | 3306 | MySQL backend for #1 |
| `wordpress-test-postgres-1` | 5432 | unused this session |

### Smoke test

`curl -sI http://localhost:8084/` → **HTTP 200 OK**, Apache + PHP 8.3.20 serving WP 6.7. The page renders.

### Real errors observed in `docker logs wordpress-test-wordpress-synapse-1`

WP renders, but the log is full of **protocol-compatibility bugs**:

1. `UNIQUE constraint failed: wp_options.option_name` on `INSERT ... ON DUPLICATE KEY UPDATE` — the rewriter does NOT translate MySQL upsert syntax to SQLite `INSERT ... ON CONFLICT`; the SQLite engine sees a plain INSERT and rejects on the unique index.
2. `Packet buffer wasn't big enough` on a large `_site_transient_wp_theme_files_patterns-...` payload — the MySQL wire-protocol packet limit is hit before the rewriter even runs.
3. `mysqli_query(): Error reading result set's header` immediately after — protocol desync.
4. `Commands out of sync; you can't run this command now` repeated — the desync from #2/#3 cascades through subsequent queries.
5. `PHP Warning: Undefined property: stdClass::$Field` — the `SHOW COLUMNS` response shape doesn't expose the columns WP expects (`Field`, `Type`).

WP is therefore **bootable but unusable for real workloads**. Reads of static pages return; writes silently fail.

## Code-level gaps in `synapse-mysql/src/rewrite.rs`

273 LoC in **a single `pub fn rewrite()`** built from `regex::Regex` rules. Coverage is heuristic, not parser-based:

- ✅ Multi-table DELETE → no-op
- ✅ `SET ...` → mostly no-op or `PRAGMA foreign_keys`
- ✅ `SHOW TABLES`, `SHOW DATABASES`, `SHOW VARIABLES LIKE`, `SHOW CREATE TABLE`, `SHOW COLUMNS` (partial, missing `Field` field shape per error #5)
- ✅ **`INSERT ... ON DUPLICATE KEY UPDATE`** — rewritten to `INSERT ... ON CONFLICT(col) DO UPDATE SET` for known WP tables; `INSERT OR REPLACE` fallback for unknown tables. Branch `wp-fix-on-duplicate`, commit `6f598e4`.
- ❌ **MySQL-specific functions** (`SQL_CALC_FOUND_ROWS`, `FOUND_ROWS()`, `IF()`, `IFNULL()` semantics differ from SQLite) — passed through unchanged.
- ❌ **DATETIME/TIMESTAMP** type coercion — SQLite has no native datetime; WP queries that compare/order on date columns will return string-sorted results.
- ❌ **`SHOW INDEX FROM`** — not in the rewriter.
- ❌ **Engine-specific options** (`ENGINE=InnoDB`, `CHARSET=utf8mb4`, `COLLATE=...` in `CREATE TABLE`) — must be stripped, currently isn't (causes silent migration failures).
- ❌ **Wire-protocol packet sizing** — error #2 is at the `msql-srv` layer, below the rewriter.

## What synapse-mysql provides correctly

- The TCP listener, the MySQL handshake, the `ColumnType` mapping for INT/REAL/BLOB/VARCHAR (via `map_type()`)
- `regexp` UDF registration so WP regex queries find a function by name
- ACL stub (`acl.rs`, 57 LoC, single user "root", grant-check stub)
- Prepared-statement cache (`stmts: HashMap<u32, String>`, `stmt_id_seq`)

## Test coverage gap

```
cargo test --release -p synapse-mysql
  → cargo test: 0 passed (1 suite, 0.00s)
```

There are **zero tests in synapse-mysql**. The 273-LoC rewriter is unverified except by manual WP boot. Compare to upstream `sqlite-database-integration-2.2.23` which ships extensive test suites (per its AGENTS.md).

## Honest verdict

| Claim | Supported? |
|---|---|
| "WordPress runs against Synapse-MySQL" | **Yes**, the container boots, page renders 200 OK |
| "WordPress is functional against Synapse-MySQL" | **No** — writes hit `UNIQUE constraint failed`, large pages cause protocol desync, follow-up queries fail with "Commands out of sync" |
| "Production-ready" | **No** — `cargo test -p synapse-mysql` has 0 tests, single-function regex rewriter, no `INSERT ON DUPLICATE KEY UPDATE` translation |
| "Better than upstream sqlite-database-integration plugin" | **No** — that plugin has a full lexer + parser + translator (3 000+ LoC of PHP per file in `wordpress-test/sqlite-database-integration-2.2.23/wp-includes/database/`), Synapse-MySQL has a 273-LoC regex rewriter |

## Concrete next-step backlog (prioritized)

1. ~~**Add `INSERT ... ON DUPLICATE KEY UPDATE → INSERT ... ON CONFLICT DO UPDATE` rewrite**~~ ✅ **FIXED** — `crates/synapse-mysql/src/rewrite.rs`, commit `6f598e4`, branch `wp-fix-on-duplicate`. 7 unit tests green. Known WP tables use hardcoded conflict column; plugin tables fall back to `INSERT OR REPLACE`. `UNIQUE constraint failed: wp_options.option_name` errors: 0 in post-restart log.
2. **Fix `SHOW COLUMNS` output shape** to expose `Field`, `Type`, `Null`, `Key`, `Default`, `Extra` columns expected by WP — kills error #5 cascade. ~40 LoC + 3 tests.
3. **Strip MySQL engine options** from `CREATE TABLE` (`ENGINE=`, `CHARSET=`, `COLLATE=`, `AUTO_INCREMENT=`) — needed for clean migration. ~50 LoC + 5 tests.
4. **Bump msql-srv packet buffer** to handle WP serialized blobs > 64 KB — kills error #2. Likely a configuration knob in the patched `msql-srv` (see `crates/msql-srv-patched/`).
5. **Translate `SQL_CALC_FOUND_ROWS` + `FOUND_ROWS()`** to SQLite-equivalent (run query without `LIMIT`, then capture `COUNT(*)`).
6. **Datetime coercion layer** — store as SQLite TEXT in ISO-8601, intercept `DATE()`, `DATETIME()`, `UNIX_TIMESTAMP()` calls, register equivalent UDFs.
7. **Adopt the upstream lexer+parser** (port to Rust or call out to PHP-WASM) — strategic; ends the regex-rewriter approach.
8. **Add cargo tests covering the WP boot SQL trace** — capture all queries WP issues during install + admin login + post creation, replay them through the rewriter, assert no panics + correct SQLite output.

## Recommendation

The synapse-mysql crate is a **proof-of-concept** that proves the wire protocol works. It should be marked `experimental`/`alpha` in workspace docs. Items 1-2-3-4 of the backlog give you a usable WP-on-Synapse for read-mostly demo scenarios. Beyond that, item 7 is the strategic decision: either Synapse owns a real MySQL parser (large undertaking), or it delegates to the existing PHP plugin and only handles the storage layer (smaller, more honest scope).

## Files

- `crates/synapse-mysql/src/server.rs` (207 LoC)
- `crates/synapse-mysql/src/rewrite.rs` (273 LoC, single regex-based fn)
- `crates/synapse-mysql/src/acl.rs` (57 LoC, stub)
- `crates/synapse-mysql/src/main.rs` (103 LoC, CLI)
- `wordpress-test/docker-compose.yml` (3 WP variants + 2 backends)
- `wordpress-test/sqlite-database-integration-2.2.23/` (upstream Adminer/W3 plugin, full PHP lexer+parser, vendored)
- `crates/msql-srv-patched/` (workspace patch of msql-srv crate)

## Anti-claims (do not market)

- "Drop-in MySQL replacement" — measured 5 distinct error families on a stock WP install in this session
- "WP-compatible" — bootable ≠ functional
- "Faster than MySQL" — n/m, never benchmarked

## Defensible claims (with current data)

- "Synapse-MySQL accepts MySQL wire protocol connections from PHP/WordPress and serves SHOW/SET/SELECT requests sufficient for WP page render." (verified: 200 OK on HEAD `/`, page HTML rendered)
- "synapse-mysql is a 640-LoC PoC sitting next to a comprehensive upstream PHP plugin (sqlite-database-integration v2.2.23) bundled in this repo." (verified)

---
Author: hyperstack-heavy (Opus 4.7), 2026-04-23. All errors quoted verbatim from `docker logs wordpress-test-wordpress-synapse-1` this session.
