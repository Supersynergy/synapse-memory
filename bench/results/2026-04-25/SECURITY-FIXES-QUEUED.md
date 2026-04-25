# Security Fixes — 2026-04-25

## CRIT-1 ✓ DONE — SQL injection via `SHOW COLUMNS FROM <table>`
**File:** `crates/synapse-mysql/src/rewrite.rs`
**Issue:** Table name was interpolated into `pragma_table_info('{}')` without
validation. A crafted name like `"evil'); DROP TABLE x; --"` would break out
of the literal and execute arbitrary SQLite.

**Fix:** Added `safe_table_name()` validator: `^[A-Za-z0-9_]+$`, len ≤ 64.
Applied at all 3 interpolation sites: `SHOW CREATE TABLE`, `SHOW (FULL) COLUMNS FROM`,
`SHOW INDEX/KEYS FROM`, and `DESC/DESCRIBE`. Invalid identifiers now return
a parse error and never reach SQLite.

**Tests added:**
- `test_safe_table_name_accepts_valid`
- `test_safe_table_name_rejects_injection` (incl. the exact PoC payload)
- `test_show_columns_rejects_injection`
- `test_show_index_rejects_injection`

## HIGH-2 ✓ DONE — `count_placeholders` miscounts inside `$$…$$`
**File:** `crates/synapse-mysql-async/src/shim.rs`
**Issue:** `count_placeholders` did not understand PostgreSQL-style dollar-quoted
tokens, so a `?` inside a `$$body$$` block could be miscounted as a parameter,
producing a wrong `n_params` and a parameter-count mismatch on prepared
statements.

**Fix:** Added `$$` skip at the top of the scanning loop (outside any quoted
region the scanner already tracks).

**Tests added:**
- `count_placeholders_basic`
- `count_placeholders_skips_dollar_dollar` (asserts `count_placeholders("select $$") == 0`)
- `count_placeholders_skips_strings_and_comments`

## TASK A — wp-cli silent exit (data-phase boundary)
**File:** `crates/synapse-mysql/src/rewrite.rs` (`SHOW VARIABLES` handler)

**Boundary query identified:** `SHOW VARIABLES LIKE 'character_set_client'`
(and the family `character_set_*`, `collation_*`, `version`, `sql_mode`).

The previous handler returned the literal var name with `Value = ''`. wpdb's
`init_charset()` reads `Value`, sees an empty string, sets `$this->charset = ''`
and silently aborts the install routine before the data-write phase. Tables
stayed empty even though SELECT/SHOW had reached the daemon.

**Fix:** New `MYSQL_DEFAULT_VARS` constant table with canonical MySQL 8.0
defaults. `SHOW VARIABLES LIKE '<glob>'` now expands the glob and emits
real `(Variable_name, Value)` rows. Bare `SHOW VARIABLES` returns the full
canonical table.

**Tests added:**
- `test_show_variables_returns_canonical_charset`
- `test_show_variables_glob_expands`

## Verification
`cargo test -p synapse-mysql -p synapse-mysql-async` → **59 passed, 0 failed**
(was 55 pre-fix; +5 new security/regression tests, +1 carried in mysql-async).
