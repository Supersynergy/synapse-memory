# WP Performance Patches — 2026-04-23

## Fix 1: CLIENT_SSL capability bit (MERGED)

**File**: `crates/msql-srv-patched/src/lib.rs`
**Commit**: fix(handshake): accept CLIENT_SSL flag without TLS impl

**Problem**: PHP/mysqlnd (PHP 8.x) unconditionally sets `CLIENT_SSL` in the client
handshake response even when `ssl_mode=DISABLED`. The proxy errored with
`"client requested SSL despite us not advertising support for it"` and dropped
the connection. This caused all WP → Synapse connections to fail (UC6, UC7, UC9
all BLOCKED).

**Change**: Removed the hard error on `CLIENT_SSL` when `tls` feature is not
compiled. The proxy now silently ignores the flag and continues in plaintext.
The cap_lo advertised is `0xa201` (no SSL bit advertised), which is correct —
we never told the client we support SSL.

**Before**: All WP write operations → BLOCKED (handshake crash)
**After**: Connections succeed, WP fully operational against Synapse

---

## Fix 2: busy_timeout PRAGMA (MERGED)

**File**: `crates/synapse-core/src/db.rs`
**Commit**: fix(db): add busy_timeout=5000ms pragma to Store::open

**Change**: Added `PRAGMA busy_timeout=5000` after WAL/synchronous pragmas.

**Why**: SQLite WAL allows concurrent readers but only one writer. Without
busy_timeout, concurrent WP requests that need writes immediately fail with
SQLITE_BUSY. With 5s timeout, requests queue and retry instead.

Other pragmas already set: `journal_mode=WAL`, `synchronous=NORMAL`,
`temp_store=MEMORY`, `mmap_size=256MB`, `cache_size=-65536` (64MB).

---

## Fix 3: Transaction batching — DEFERRED

**Reason**: After Fix 1+2, UC9 (100x post meta updates) takes ~19s. Profiling
shows the bottleneck is `docker exec wp ... post meta update` overhead
(~196ms/exec × 100 = 19.6s). This is bench methodology overhead, not SQLite
transaction overhead. Each `docker exec` spawns a new PHP process and new
MySQL connection — autocommit batching would not reduce this.

**Real-world impact**: Direct WP HTTP requests that trigger batch meta updates
within a single PHP request would benefit from batching. Deferred until a
HTTP-driven bench can measure this properly.

---

## Bench Infrastructure Fixes (run.sh)

- Fixed `date +%s%3N` (Linux-only) → `ms_now()` using `python3 -c` (macOS-portable)
- Installed wp-cli in `wp3_wp_c` (Synapse container) for UC6/7/8/9
- Ran `wp core install` for wp_c (Synapse) which was uninitialized

---

## Honest Limits

- Percona8 remains DOWN (container health failure, unrelated to this work)
- MySQL8 UC6/7/8/9 also BLOCKED (no wp-cli in those containers) — not a Synapse regression
- Synapse is ~3-4× slower than MySQL8 on HTTP response times (network stack overhead, not SQLite)
- UC9 wall time dominated by bench methodology, not DB performance
