# WP E2E Install — synapse-mysql-async
**Date**: 2026-04-25  
**WP Version**: 6.5  
**Synapse**: synapse-mysql-async (mode=medium-coeli, file=/tmp/wp-real.db)

---

## Install: FAIL

### Exact Error (wp-cli output)
```
Error: Error establishing a database connection. This either means that the
username and password information in your wp-config.php file is incorrect or
that contact with the database server at 127.0.0.1:13317 could not be
established.
```

### Root Cause (pymysql probe)
WP install calls `SELECT DATABASE()` early in its DB setup sequence.
`synapse-mysql-async` responds with a **connection drop** (errno 2013:
"Lost connection to MySQL server during query"), causing WP to abort
with a generic "can't connect" error.

---

## Top-3 Missing MySQL Features

| # | Query | Status | Impact |
|---|-------|--------|--------|
| 1 | `SELECT DATABASE()` | **CONNECTION DROP** | WP calls this to verify active DB; drop kills session |
| 2 | `SELECT LAST_INSERT_ID()` | **CONNECTION DROP** | WP calls after every INSERT for new post/option IDs |
| 3 | `SHOW FULL COLUMNS FROM <table>` | Returns empty (`[]`) | WP uses this for schema migration checks; silent breakage |

---

## What Passed (surprising)
- Basic auth handshake: OK
- `CREATE DATABASE`, `USE db`, `SHOW TABLES`: OK
- `CREATE TABLE` (InnoDB, AUTO_INCREMENT, UNIQUE KEY): OK
- `INSERT` + `SELECT`: OK (rows persist)
- `SHOW CHARACTER SET`, `SHOW COLLATION`: OK
- `SET NAMES utf8mb4`, `SET SESSION sql_mode`: OK
- `SELECT VERSION()` → `8.0.30-synapse`: OK

---

## TTFB
Not measurable — WP never completed install. No PHP server was started.

---

## Verdict: Drop-In Achievable Today?

**No.** Two showstopper panics:

1. `SELECT DATABASE()` — Standard MySQL function, called in WP's `wpdb::select()` to confirm DB context. Connection drop means WP sees a dead connection and aborts.
2. `SELECT LAST_INSERT_ID()` — Called after every successful INSERT in WP's `wpdb::insert()`. Without this, WP can't retrieve new post IDs, option IDs, or user IDs.

Both must return valid results (not crash the connection) before WP install can proceed. These are 2-line SQL stubs in synapse-mysql-async — high priority, low complexity fixes.

**Fix path**: Implement `SELECT DATABASE()` (return current db context string) and `SELECT LAST_INSERT_ID()` (return last rowid from SQLite `last_insert_rowid()`). ETA: ~1h implementation + retest.
